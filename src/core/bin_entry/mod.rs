use self::{tree_bin::TreeBin, tree_node::TreeNode};

use super::node::Node;

pub mod tree_bin;
pub mod tree_node;

#[derive(Debug)]
pub enum BinEntry<K, V> {
    Node(Node<K, V>),
    Tree(TreeBin<K, V>),
    TreeNode(TreeNode<K, V>),
    Moved,
}

impl<K, V> BinEntry<K, V> {
    pub fn as_node(&self) -> Option<&Node<K, V>> {
        if let BinEntry::Node(ref n) = *self {
            Some(n)
        } else {
            None
        }
    }

    pub fn as_tree_node(&self) -> Option<&TreeNode<K, V>> {
        if let BinEntry::TreeNode(ref n) = *self {
            Some(n)
        } else {
            None
        }
    }

    pub fn as_tree_bin(&self) -> Option<&TreeBin<K, V>> {
        if let BinEntry::Tree(ref n) = *self {
            Some(n)
        } else {
            None
        }
    }
}
