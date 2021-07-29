use std::{
    borrow::Borrow,
    sync::atomic::{AtomicBool, Ordering},
};

use crossbeam_epoch::{Atomic, Guard, Shared};

use crate::core::node::Node;

use super::BinEntry;

#[derive(Debug)]
pub struct TreeNode<K, V> {
    pub node: Node<K, V>,
    pub parent: Atomic<BinEntry<K, V>>,
    pub left: Atomic<BinEntry<K, V>>,
    pub right: Atomic<BinEntry<K, V>>,
    pub prev: Atomic<BinEntry<K, V>>,
    pub red: AtomicBool,
}

impl<K, V> TreeNode<K, V> {
    pub fn new(
        hash: u64,
        key: K,
        value: Atomic<V>,
        next: Atomic<BinEntry<K, V>>,
        parent: Atomic<BinEntry<K, V>>,
    ) -> Self {
        TreeNode {
            node: Node::new(hash, key, value, next),
            parent,
            left: Atomic::null(),
            right: Atomic::null(),
            prev: Atomic::null(),
            red: AtomicBool::new(false),
        }
    }

    pub fn find_tree_node<'t, Q>(
        from: Shared<'t, BinEntry<K, V>>,
        hash: u64,
        key: &Q,
        guard: &'t Guard,
    ) -> Shared<'t, BinEntry<K, V>>
    where
        K: Borrow<Q>,
        Q: ?Sized + Ord,
    {
        let mut p = from;
        while p.is_null() == false {
            let p_deref = unsafe { Self::get_tree_node(p) };
            let p_hash = p_deref.node.hash;

            match p_hash.cmp(&hash) {
                std::cmp::Ordering::Greater => {
                    p = p_deref.left.load(Ordering::SeqCst, guard);
                    continue;
                }
                std::cmp::Ordering::Less => {
                    p = p_deref.right.load(Ordering::SeqCst, guard);
                    continue;
                }
                _ => {}
            }

            let p_key = &p_deref.node.key;
            if p_key.borrow() == key {
                return p;
            }

            let p_left = p_deref.left.load(Ordering::SeqCst, guard);
            let p_right = p_deref.right.load(Ordering::SeqCst, guard);

            if p_left.is_null() || p_right.is_null() {
                p = if p_left.is_null() { p_right } else { p_left };
                continue;
            }

            p = match p_key.borrow().cmp(key) {
                std::cmp::Ordering::Greater => p_left,
                std::cmp::Ordering::Less => p_right,
                _ => unreachable!(),
            }
        }
        Shared::null()
    }

    pub fn balance_insertion<'t>(
        mut root: Shared<'t, BinEntry<K, V>>,
        mut x: Shared<'t, BinEntry<K, V>>,
        guard: &'t Guard,
    ) -> Shared<'t, BinEntry<K, V>> {
        #[inline]
        fn get_red<'l, K, V>(x: Shared<'l, BinEntry<K, V>>) -> &'l AtomicBool {
            &unsafe { TreeNode::get_tree_node(x) }.red
        }

        get_red(x).store(true, Ordering::Relaxed);

        let mut x_parent: Shared<'_, BinEntry<K, V>>;
        let mut x_parent_parent: Shared<'_, BinEntry<K, V>>;
        let mut x_parent_parent_left: Shared<'_, BinEntry<K, V>>;
        let mut x_parent_parent_right: Shared<'_, BinEntry<K, V>>;

        loop {
            x_parent = unsafe { Self::get_tree_node(x) }
                .parent
                .load(Ordering::Relaxed, guard);

            if x_parent.is_null() {
                get_red(x).store(false, Ordering::Relaxed);
                return x;
            }

            x_parent_parent = unsafe { Self::get_tree_node(x_parent) }
                .parent
                .load(Ordering::Relaxed, guard);

            if get_red(x_parent).load(Ordering::Relaxed) == false || x_parent_parent.is_null() {
                return root;
            }

            x_parent_parent_left = unsafe { Self::get_tree_node(x_parent_parent) }
                .left
                .load(Ordering::Relaxed, guard);

            if x_parent == x_parent_parent_left {
                x_parent_parent_right = unsafe { Self::get_tree_node(x_parent_parent) }
                    .right
                    .load(Ordering::Relaxed, guard);

                if x_parent_parent_right.is_null() == false
                    && get_red(x_parent_parent_right).load(Ordering::Relaxed)
                {
                    get_red(x_parent_parent_right).store(false, Ordering::Relaxed);

                    get_red(x_parent).store(false, Ordering::Relaxed);

                    get_red(x_parent_parent).store(true, Ordering::Relaxed);

                    x = x_parent_parent;
                } else {
                    if x == unsafe { Self::get_tree_node(x_parent) }
                        .right
                        .load(Ordering::Relaxed, guard)
                    {
                        x = x_parent;
                        root = Self::rotate_left(root, x, guard);
                        x_parent = unsafe { Self::get_tree_node(x) }
                            .parent
                            .load(Ordering::Relaxed, guard);

                        x_parent_parent = if x_parent.is_null() {
                            Shared::null()
                        } else {
                            unsafe { Self::get_tree_node(x_parent) }
                                .parent
                                .load(Ordering::Relaxed, guard)
                        };
                    }

                    if x_parent.is_null() == false {
                        get_red(x_parent).store(false, Ordering::Relaxed);

                        if x_parent_parent.is_null() == false {
                            get_red(x_parent_parent).store(true, Ordering::Relaxed);
                            root = Self::rotate_right(root, x_parent_parent, guard);
                        }
                    }
                }
            } else if x_parent_parent_left.is_null() == false
                && get_red(x_parent_parent).load(Ordering::Relaxed)
            {
                get_red(x_parent_parent_left).store(false, Ordering::Relaxed);
                get_red(x_parent).store(false, Ordering::Relaxed);
                get_red(x_parent_parent).store(true, Ordering::Relaxed);
                x = x_parent_parent;
            } else {
                if x == unsafe { Self::get_tree_node(x_parent) }
                    .left
                    .load(Ordering::Relaxed, guard)
                {
                    x = x_parent;
                    root = Self::rotate_right(root, x, guard);
                    x_parent = unsafe { Self::get_tree_node(x) }
                        .parent
                        .load(Ordering::Relaxed, guard);

                    x_parent_parent = if x_parent.is_null() {
                        Shared::null()
                    } else {
                        unsafe { Self::get_tree_node(x_parent) }
                            .parent
                            .load(Ordering::Relaxed, guard)
                    };
                }
                if x_parent.is_null() == false {
                    get_red(x_parent).store(false, Ordering::Relaxed);
                    if x_parent_parent.is_null() == false {
                        get_red(x_parent_parent).store(true, Ordering::Relaxed);
                        root = Self::rotate_left(root, x_parent_parent, guard);
                    }
                }
            }
        }
    }

    fn rotate_left<'l>(
        mut root: Shared<'l, BinEntry<K, V>>,
        p: Shared<'l, BinEntry<K, V>>,
        guard: &'l Guard,
    ) -> Shared<'l, BinEntry<K, V>> {
        if p.is_null() {
            return root;
        }

        let p_deref = unsafe { Self::get_tree_node(p) };
        let right = p_deref.right.load(Ordering::Relaxed, guard);

        if right.is_null() {
            return root;
        }

        let right_deref = unsafe { Self::get_tree_node(right) };
        let right_left = right_deref.left.load(Ordering::Relaxed, guard);
        p_deref.right.store(right_left, Ordering::Relaxed);

        if right_left.is_null() == false {
            unsafe { Self::get_tree_node(right_left) }
                .parent
                .store(p, Ordering::Relaxed);
        }

        let p_parent = p_deref.parent.load(Ordering::Relaxed, guard);
        right_deref.parent.store(p_parent, Ordering::Relaxed);

        if p_parent.is_null() {
            root = right;
            right_deref.red.store(false, Ordering::Relaxed);
        } else {
            let p_parent_deref = unsafe { Self::get_tree_node(p_parent) };

            if p_parent_deref.left.load(Ordering::Relaxed, guard) == p {
                p_parent_deref.left.store(right, Ordering::Relaxed);
            } else {
                p_parent_deref.right.store(right, Ordering::Relaxed);
            }
        }

        right_deref.left.store(p, Ordering::Relaxed);
        p_deref.parent.store(right, Ordering::Relaxed);

        root
    }

    fn rotate_right<'l>(
        mut root: Shared<'l, BinEntry<K, V>>,
        p: Shared<'l, BinEntry<K, V>>,
        guard: &'l Guard,
    ) -> Shared<'l, BinEntry<K, V>> {
        if p.is_null() {
            return root;
        }

        let p_deref = unsafe { Self::get_tree_node(p) };
        let left = p_deref.left.load(Ordering::Relaxed, guard);

        if left.is_null() {
            return root;
        }

        let left_deref = unsafe { Self::get_tree_node(left) };
        let left_right = left_deref.right.load(Ordering::Relaxed, guard);
        p_deref.left.store(left_right, Ordering::Relaxed);

        if left_right.is_null() == false {
            unsafe { Self::get_tree_node(left_right) }
                .parent
                .store(p, Ordering::Relaxed);
        }

        let p_parent = p_deref.parent.load(Ordering::Relaxed, guard);
        left_deref.parent.store(p_parent, Ordering::Relaxed);

        if p_parent.is_null() {
            root = left;
            left_deref.red.store(false, Ordering::Relaxed);
        } else {
            let p_parent_deref = unsafe { Self::get_tree_node(p_parent) };
            if p_parent_deref.right.load(Ordering::Relaxed, guard) == p {
                p_parent_deref.right.store(left, Ordering::Relaxed);
            } else {
                p_parent_deref.left.store(left, Ordering::Relaxed);
            }
        }

        left_deref.right.store(p, Ordering::Relaxed);
        p_deref.parent.store(left, Ordering::Relaxed);

        root
    }

    pub fn balance_deletion<'l>(
        mut root: Shared<'l, BinEntry<K, V>>,
        mut x: Shared<'l, BinEntry<K, V>>,
        guard: &Guard,
    ) -> Shared<'l, BinEntry<K, V>> {
        todo!()
    }

    pub unsafe fn get_tree_node(bin: Shared<'_, BinEntry<K, V>>) -> &'_ TreeNode<K, V> {
        bin.deref().as_tree_node().unwrap()
    }
}
