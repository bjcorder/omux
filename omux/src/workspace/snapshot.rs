//! Serde-friendly mirror of [`crate::pane::tree::PaneTree`] for persistence.
//!
//! The runtime tree holds GTK widgets and is not serializable. This module
//! defines a parallel, plain-data representation that round-trips through
//! TOML and can be rebuilt into a fresh `PaneTree` on workspace load.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    /// Panes laid out side-by-side ("h-split").
    Horizontal,
    /// Panes stacked top/bottom ("v-split").
    Vertical,
}

impl Orientation {
    pub fn to_gtk(self) -> gtk4::Orientation {
        match self {
            Orientation::Horizontal => gtk4::Orientation::Horizontal,
            Orientation::Vertical => gtk4::Orientation::Vertical,
        }
    }
    pub fn from_gtk(o: gtk4::Orientation) -> Self {
        match o {
            gtk4::Orientation::Vertical => Orientation::Vertical,
            _ => Orientation::Horizontal,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LayoutNode {
    Leaf(LeafSnapshot),
    Split(SplitSnapshot),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LeafSnapshot {
    pub id: Uuid,
    /// How many tabs the leaf had. Each tab gets a fresh shell on restore
    /// (PTYs are not persistable).
    #[serde(default = "default_tab_count")]
    pub tabs: usize,
}

fn default_tab_count() -> usize {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SplitSnapshot {
    pub orientation: Orientation,
    /// Split position in pixels (gtk::Paned::position). 0 means "use the
    /// widget's natural ratio".
    #[serde(default)]
    pub position: i32,
    pub a: Box<LayoutNode>,
    pub b: Box<LayoutNode>,
}

impl LayoutNode {
    pub fn single_leaf() -> Self {
        LayoutNode::Leaf(LeafSnapshot {
            id: Uuid::new_v4(),
            tabs: 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_single_leaf() {
        let node = LayoutNode::single_leaf();
        let s = toml::to_string(&node).unwrap();
        let back: LayoutNode = toml::from_str(&s).unwrap();
        assert_eq!(node, back);
    }

    #[test]
    fn round_trip_nested_split() {
        let a = LayoutNode::Leaf(LeafSnapshot {
            id: Uuid::nil(),
            tabs: 2,
        });
        let b = LayoutNode::Split(SplitSnapshot {
            orientation: Orientation::Vertical,
            position: 200,
            a: Box::new(LayoutNode::Leaf(LeafSnapshot {
                id: Uuid::from_u128(1),
                tabs: 1,
            })),
            b: Box::new(LayoutNode::Leaf(LeafSnapshot {
                id: Uuid::from_u128(2),
                tabs: 3,
            })),
        });
        let root = LayoutNode::Split(SplitSnapshot {
            orientation: Orientation::Horizontal,
            position: 600,
            a: Box::new(a),
            b: Box::new(b),
        });

        let s = toml::to_string(&root).unwrap();
        let back: LayoutNode = toml::from_str(&s).unwrap();
        assert_eq!(root, back);
    }
}
