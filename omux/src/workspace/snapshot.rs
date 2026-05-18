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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TabKind {
    Terminal,
    Browser,
    Scratchpad,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TabSnapshot {
    pub kind: TabKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl TabSnapshot {
    pub fn terminal() -> Self {
        Self {
            kind: TabKind::Terminal,
            url: None,
        }
    }

    pub fn browser(url: Option<String>) -> Self {
        Self {
            kind: TabKind::Browser,
            url,
        }
    }

    pub fn scratchpad() -> Self {
        Self {
            kind: TabKind::Scratchpad,
            url: None,
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
    /// Per-tab kind + url. Deserializes from either a legacy integer
    /// (treated as N terminal tabs) or a list, so workspaces created
    /// before M5 still load.
    #[serde(default = "default_tabs", deserialize_with = "deserialize_tabs")]
    pub tabs: Vec<TabSnapshot>,
}

fn default_tabs() -> Vec<TabSnapshot> {
    vec![TabSnapshot::terminal()]
}

fn deserialize_tabs<'de, D>(deserializer: D) -> Result<Vec<TabSnapshot>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Count(usize),
        List(Vec<TabSnapshot>),
    }

    Ok(match Either::deserialize(deserializer)? {
        Either::Count(n) => (0..n.max(1)).map(|_| TabSnapshot::terminal()).collect(),
        Either::List(v) if v.is_empty() => default_tabs(),
        Either::List(v) => v,
    })
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
            tabs: default_tabs(),
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
    fn round_trip_nested_split_with_mixed_tabs() {
        let mixed_leaf = LayoutNode::Leaf(LeafSnapshot {
            id: Uuid::from_u128(7),
            tabs: vec![
                TabSnapshot::terminal(),
                TabSnapshot::browser(Some("https://docs.rs/".into())),
            ],
        });
        let root = LayoutNode::Split(SplitSnapshot {
            orientation: Orientation::Horizontal,
            position: 600,
            a: Box::new(LayoutNode::Leaf(LeafSnapshot {
                id: Uuid::from_u128(1),
                tabs: vec![TabSnapshot::terminal()],
            })),
            b: Box::new(mixed_leaf),
        });
        let s = toml::to_string(&root).unwrap();
        let back: LayoutNode = toml::from_str(&s).unwrap();
        assert_eq!(root, back);
    }

    #[test]
    fn round_trip_scratchpad_tab_without_content() {
        let root = LayoutNode::Leaf(LeafSnapshot {
            id: Uuid::from_u128(9),
            tabs: vec![TabSnapshot::scratchpad()],
        });

        let s = toml::to_string(&root).unwrap();

        assert!(s.contains("kind = \"scratchpad\""));
        assert!(!s.contains("content"));
        let back: LayoutNode = toml::from_str(&s).unwrap();
        assert_eq!(root, back);
    }

    #[test]
    fn deserializes_scratchpad_tab_kind() {
        let raw = r#"
            kind = "leaf"
            id = "00000000-0000-0000-0000-000000000001"
            [[tabs]]
            kind = "scratchpad"
        "#;

        let node: LayoutNode = toml::from_str(raw).unwrap();

        let LayoutNode::Leaf(leaf) = node else {
            panic!("expected leaf")
        };
        assert_eq!(leaf.tabs.len(), 1);
        assert_eq!(leaf.tabs[0].kind, TabKind::Scratchpad);
        assert_eq!(leaf.tabs[0].url, None);
    }

    #[test]
    fn deserializes_legacy_tabs_integer() {
        let raw = r#"
            kind = "leaf"
            id = "00000000-0000-0000-0000-000000000001"
            tabs = 3
        "#;
        let node: LayoutNode = toml::from_str(raw).unwrap();
        let LayoutNode::Leaf(leaf) = node else {
            panic!("expected leaf")
        };
        assert_eq!(leaf.tabs.len(), 3);
        assert!(leaf.tabs.iter().all(|t| t.kind == TabKind::Terminal));
    }

    #[test]
    fn empty_tabs_falls_back_to_one_terminal() {
        let raw = r#"
            kind = "leaf"
            id = "00000000-0000-0000-0000-000000000001"
            tabs = []
        "#;
        let node: LayoutNode = toml::from_str(raw).unwrap();
        let LayoutNode::Leaf(leaf) = node else {
            panic!("expected leaf")
        };
        assert_eq!(leaf.tabs.len(), 1);
        assert_eq!(leaf.tabs[0].kind, TabKind::Terminal);
    }
}
