use std::{
    borrow::Borrow,
    env::consts::FAMILY,
    sync::atomic::{AtomicI64, Ordering},
    thread::{current, park, Thread},
};

use crossbeam_epoch::{Atomic, Guard, Owned, Shared};

use crate::util::{dir::Dir, state::State};

use super::{tree_node::TreeNode, BinEntry};

#[derive(Debug)]
pub struct TreeBin<K, V> {
    pub root: Atomic<BinEntry<K, V>>,
    pub first: Atomic<BinEntry<K, V>>,
    pub waiter: Atomic<Thread>,
    pub lock: parking_lot::Mutex<()>,
    pub lock_state: AtomicI64,
}

impl<K, V> TreeBin<K, V>
where
    K: Ord,
{
    pub fn new(bin: Owned<BinEntry<K, V>>, guard: &Guard) -> Self {
        let mut root = Shared::null();
        let bin = bin.into_shared(guard);

        let mut x = bin;
        while x.is_null() == false {
            let x_deref = unsafe { TreeNode::get_tree_node(x) };
            let next = x_deref.node.next.load(Ordering::Relaxed, guard);

            x_deref.left.store(Shared::null(), Ordering::Relaxed);
            x_deref.right.store(Shared::null(), Ordering::Relaxed);

            if root.is_null() {
                x_deref.parent.store(Shared::null(), Ordering::Relaxed);
                x_deref.red.store(false, Ordering::Relaxed);
                root = x;
                x = next;
                continue;
            }

            let key = &x_deref.node.key;
            let hash = x_deref.node.hash;

            let mut p = root;
            loop {
                let p_deref = unsafe { TreeNode::get_tree_node(p) };
                let p_key = &p_deref.node.key;
                let p_hash = p_deref.node.hash;

                let xp = p;
                let dir: Dir;
                p = match p_hash.cmp(&hash).then(p_key.cmp(key)) {
                    std::cmp::Ordering::Greater => {
                        dir = Dir::Left;
                        &p_deref.left
                    }
                    std::cmp::Ordering::Less => {
                        dir = Dir::Right;
                        &p_deref.right
                    }
                    std::cmp::Ordering::Equal => unreachable!(),
                }
                .load(Ordering::Relaxed, guard);

                if p.is_null() {
                    x_deref.parent.store(xp, Ordering::Relaxed);
                    match dir {
                        Dir::Left => unsafe { TreeNode::get_tree_node(xp) }
                            .left
                            .store(x, Ordering::Relaxed),
                        Dir::Right => unsafe { TreeNode::get_tree_node(xp) }
                            .right
                            .store(x, Ordering::Relaxed),
                    }
                }

                root = TreeNode::balance_insertion(root, x, guard);
                break;
            }
        }

        TreeBin {
            root: Atomic::from(root),
            first: Atomic::from(bin),
            waiter: Atomic::null(),
            lock: parking_lot::Mutex::new(()),
            lock_state: AtomicI64::new(State::None as i64),
        }
    }

    fn lock_root(&self, guard: &Guard) {
        if self
            .lock_state
            .compare_exchange(
                State::None as i64,
                State::Writer as i64,
                Ordering::SeqCst,
                Ordering::Relaxed,
            )
            .is_err()
        {
            self.contended_lock(guard);
        }
    }

    fn unlock_root(&self) {
        self.lock_state.store(State::None as i64, Ordering::Release);
    }

    fn contended_lock(&self, guard: &Guard) {
        let mut waiting = false;
        let mut state: i64;

        loop {
            state = self.lock_state.load(Ordering::Acquire);
            if state & !(State::Waiter as i64) == 0 {
                if self
                    .lock_state
                    .compare_exchange(
                        state,
                        State::Writer as i64,
                        Ordering::SeqCst,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    if waiting {
                        let waiter = self.waiter.swap(Shared::null(), Ordering::SeqCst, guard);

                        unsafe { guard.defer_destroy(waiter) };
                    }
                    return;
                }
            } else if state & State::Writer as i64 == 0 {
                if self
                    .lock_state
                    .compare_exchange(
                        state,
                        state | State::Waiter as i64,
                        Ordering::SeqCst,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    waiting = true;
                    let current_thread = Owned::new(current());
                    let waiter = self.waiter.swap(current_thread, Ordering::SeqCst, guard);
                    assert!(waiter.is_null());
                }
            } else if waiting {
                park();
            }
            std::hint::spin_loop();
        }
    }

    pub fn find<'l, Q>(
        bin: Shared<'l, BinEntry<K, V>>,
        hash: u64,
        key: &Q,
        guard: &'l Guard,
    ) -> Shared<'l, BinEntry<K, V>>
    where
        K: Borrow<Q>,
        Q: ?Sized + Ord,
    {
        let bin_deref = unsafe { bin.deref() }.as_tree_bin().unwrap();
        let mut element = bin_deref.first.load(Ordering::SeqCst, guard);
        while element.is_null() == false {
            let s = bin_deref.lock_state.load(Ordering::SeqCst);
            if s & (State::Waiter as i64 | State::Writer as i64) == 0 {
                let element_deref = unsafe { TreeNode::get_tree_node(element) };
                let element_key = &element_deref.node.key;

                if element_deref.node.hash == hash && element_key.borrow() == key {
                    return element;
                }

                element = element_deref.node.next.load(Ordering::SeqCst, guard);
            } else if bin_deref
                .lock_state
                .compare_exchange(
                    s,
                    s + State::Reader as i64,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                let root = bin_deref.root.load(Ordering::SeqCst, guard);
                let p = if root.is_null() {
                    Shared::null()
                } else {
                    TreeNode::find_tree_node(root, hash, key, guard)
                };

                if bin_deref
                    .lock_state
                    .fetch_add(-(State::Reader as i64), Ordering::SeqCst)
                    == (State::Reader as i64 | State::Writer as i64)
                {
                    let waiter = &bin_deref.waiter.load(Ordering::SeqCst, guard);

                    if waiter.is_null() == false {
                        unsafe { waiter.deref() }.unpark()
                    }
                }
                return p;
            }
        }
        Shared::null()
    }

    pub unsafe fn remove_tree_node<'l>(
        &'l self,
        p: Shared<'l, BinEntry<K, V>>,
        drop_value: bool,
        guard: &'l Guard,
    ) -> bool {
        let p_deref = TreeNode::get_tree_node(p);
        let next = p_deref.node.next.load(Ordering::SeqCst, guard);
        let prev = p_deref.prev.load(Ordering::SeqCst, guard);

        if prev.is_null() {
            self.first.store(next, Ordering::SeqCst);
        } else {
            TreeNode::get_tree_node(prev)
                .node
                .next
                .store(next, Ordering::SeqCst);
        }

        if next.is_null() == false {
            TreeNode::get_tree_node(next)
                .prev
                .store(prev, Ordering::SeqCst);
        }

        if self.first.load(Ordering::SeqCst, guard).is_null() {
            self.root.store(Shared::null(), Ordering::SeqCst);
            return true;
        }

        let mut root = self.root.load(Ordering::SeqCst, guard);

        if root.is_null()
            || TreeNode::get_tree_node(root)
                .right
                .load(Ordering::SeqCst, guard)
                .is_null()
        {
            return true;
        }

        self.lock_root(guard);

        let replacement;
        let p_left = p_deref.left.load(Ordering::Relaxed, guard);
        let p_right = p_deref.right.load(Ordering::Relaxed, guard);

        if p_left.is_null() == false && p_right.is_null() == false {
            let mut succ = p_right;
            let mut succ_deref = TreeNode::get_tree_node(succ);
            let mut succ_left = succ_deref.left.load(Ordering::Relaxed, guard);

            while succ_left.is_null() == false {
                succ = succ_left;
                succ_deref = TreeNode::get_tree_node(succ);
                succ_left = succ_deref.left.load(Ordering::Relaxed, guard);
            }

            let color = succ_deref.red.load(Ordering::Relaxed);
            succ_deref
                .red
                .store(p_deref.red.load(Ordering::Relaxed), Ordering::Relaxed);

            p_deref.red.store(color, Ordering::Relaxed);

            let succ_right = succ_deref.right.load(Ordering::Relaxed, guard);
            let p_parent = p_deref.parent.load(Ordering::Relaxed, guard);

            if succ == p_right {
                p_deref.parent.store(succ, Ordering::Relaxed);
                succ_deref.right.store(p, Ordering::Relaxed);
            } else {
                let succ_parent = succ_deref.parent.load(Ordering::Relaxed, guard);
                p_deref.parent.store(succ_parent, Ordering::Relaxed);
                if succ_parent.is_null() == false {
                    if succ
                        == TreeNode::get_tree_node(succ_parent)
                            .left
                            .load(Ordering::Relaxed, guard)
                    {
                        TreeNode::get_tree_node(succ_parent)
                            .left
                            .store(p, Ordering::Relaxed);
                    } else {
                        TreeNode::get_tree_node(succ_parent)
                            .right
                            .store(p, Ordering::Relaxed);
                    }
                }
                succ_deref.right.store(p_right, Ordering::Relaxed);

                if p_right.is_null() == false {
                    TreeNode::get_tree_node(p_right)
                        .parent
                        .store(succ, Ordering::Relaxed);
                }
            }
            p_deref.left.store(Shared::null(), Ordering::Relaxed);
            p_deref.right.store(succ_right, Ordering::Relaxed);

            if succ_right.is_null() == false {
                TreeNode::get_tree_node(p_left)
                    .parent
                    .store(succ, Ordering::Relaxed);
            }

            succ_deref.left.store(p_left, Ordering::Relaxed);
            if p_left.is_null() == false {
                TreeNode::get_tree_node(p_left)
                    .parent
                    .store(succ, Ordering::Relaxed);
            }

            succ_deref.parent.store(p_parent, Ordering::Relaxed);

            if p_parent.is_null() {
                root = succ;
            } else if p
                == TreeNode::get_tree_node(p_parent)
                    .left
                    .load(Ordering::Relaxed, guard)
            {
                TreeNode::get_tree_node(p_parent)
                    .left
                    .store(succ, Ordering::Relaxed);
            } else {
                TreeNode::get_tree_node(p_parent)
                    .right
                    .store(succ, Ordering::Relaxed);
            }

            if succ_right.is_null() == false {
                replacement = succ_right;
            } else {
                replacement = p;
            }
        } else if p_left.is_null() == false {
            replacement = p_left;
        } else if p_right.is_null() == false {
            replacement = p_right;
        } else {
            replacement = p;
        }

        if replacement != p {
            let p_parent = p_deref.parent.load(Ordering::Relaxed, guard);
            TreeNode::get_tree_node(replacement)
                .parent
                .store(p_parent, Ordering::Relaxed);

            if p_parent.is_null() {
                root = replacement;
            } else {
                let p_parent_deref = TreeNode::get_tree_node(p_parent);

                if p == p_parent_deref.left.load(Ordering::Relaxed, guard) {
                    p_parent_deref.left.store(replacement, Ordering::Relaxed);
                } else {
                    p_parent_deref.right.store(replacement, Ordering::Relaxed);
                }
            }

            p_deref.parent.store(Shared::null(), Ordering::Relaxed);
            p_deref.left.store(Shared::null(), Ordering::Relaxed);
            p_deref.right.store(Shared::null(), Ordering::Relaxed);
        }

        let new_root = if p_deref.red.load(Ordering::Relaxed) {
            root
        } else {
            TreeNode::balance_deletion(root, replacement, guard)
        };

        self.root.store(new_root, Ordering::Relaxed);

        if p == replacement {
            let p_parent = p_deref.parent.load(Ordering::Relaxed, guard);

            if p_parent.is_null() == false {
                let p_parent_deref = TreeNode::get_tree_node(p_parent);

                if p == p_parent_deref.left.load(Ordering::Relaxed, guard) {
                    TreeNode::get_tree_node(p_parent)
                        .left
                        .store(Shared::null(), Ordering::Relaxed);
                } else if p == p_parent_deref.right.load(Ordering::Relaxed, guard) {
                    p_parent_deref
                        .right
                        .store(Shared::null(), Ordering::Relaxed);
                }
                p_deref.parent.store(Shared::null(), Ordering::Relaxed);
            }
        }

        self.unlock_root();

        unsafe {
            if drop_value {
                guard.defer_destroy(p_deref.node.value.load(Ordering::Relaxed, guard));
            }
            guard.defer_destroy(p);
        }

        false
    }
}
