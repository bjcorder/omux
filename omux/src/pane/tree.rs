//! Recursive pane layout tree.
//!
//! A `PaneTree` is a recursive binary tree:
//!
//! * a **leaf** is a [`gtk4::Notebook`] holding one or more
//!   [`TerminalPane`] tabs;
//! * a **split** is a [`gtk4::Paned`] (horizontal or vertical) whose two
//!   children are themselves leaves or splits.
//!
//! The tree mounts into an [`adw::Bin`] so the top-level widget can be
//! swapped out when the root structure changes (e.g. the first split
//! promotes the root from a Notebook to a Paned).
//!
//! Direction conventions (see also design §5.3):
//!
//! | gtk4 orientation | visual effect       | mnemonic |
//! |------------------|---------------------|----------|
//! | `Horizontal`     | panes side-by-side  | "h-split" |
//! | `Vertical`       | panes top/bottom    | "v-split" |

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Notebook, Orientation, Paned, Widget};
use libadwaita as adw;
use libadwaita::prelude::*;
use uuid::Uuid;

use super::terminal::TerminalPane;
use crate::agent::manifest::CompiledManifest;
use crate::workspace::SnapshotOrientation;
use crate::workspace::snapshot::{LayoutNode, LeafSnapshot, SplitSnapshot};

pub type LeafId = Uuid;

#[derive(Clone, Copy)]
enum ChildSlot {
    Start,
    End,
}

#[derive(Clone)]
pub struct PaneTree {
    bin: adw::Bin,
    state: Rc<RefCell<TreeState>>,
}

struct TreeState {
    root: NodeSlot,
    focused: LeafId,
    manifests: Rc<Vec<CompiledManifest>>,
}

type NodeSlot = Rc<RefCell<Node>>;

enum Node {
    Leaf(Leaf),
    Split(SplitNode),
}

struct Leaf {
    id: LeafId,
    notebook: Notebook,
    tabs: Vec<TerminalPane>,
}

struct SplitNode {
    paned: Paned,
    a: NodeSlot,
    b: NodeSlot,
}

impl Node {
    #[allow(dead_code)]
    fn widget(&self) -> Widget {
        match self {
            Node::Leaf(l) => l.notebook.clone().upcast(),
            Node::Split(s) => s.paned.clone().upcast(),
        }
    }
}

impl Leaf {
    fn new(manifests: &[CompiledManifest]) -> Self {
        Self::with_id_and_tabs(Uuid::new_v4(), 1, manifests)
    }

    fn with_id_and_tabs(id: Uuid, tab_count: usize, manifests: &[CompiledManifest]) -> Self {
        let notebook = Notebook::builder()
            .scrollable(true)
            .show_border(false)
            .build();
        let mut leaf = Self {
            id,
            notebook,
            tabs: Vec::new(),
        };
        for _ in 0..tab_count.max(1) {
            leaf.add_tab(manifests);
        }
        leaf
    }

    fn add_tab(&mut self, manifests: &[CompiledManifest]) {
        let pane = TerminalPane::new_with_manifests(manifests);
        let label = gtk4::Label::new(Some("shell"));
        let idx = self.notebook.append_page(pane.widget(), Some(&label));
        self.notebook.set_current_page(Some(idx));
        pane.terminal().grab_focus();
        self.tabs.push(pane);
    }
}

impl PaneTree {
    /// Build an empty tree with a single leaf containing one tab. Use
    /// [`PaneTree::from_snapshot`] for the production path (it handles
    /// the single-leaf case via [`LayoutNode::single_leaf`]).
    #[allow(dead_code)]
    pub fn new(manifests: &[CompiledManifest]) -> Self {
        let leaf = Leaf::new(manifests);
        let focused = leaf.id;
        let root_widget: Widget = leaf.notebook.clone().upcast();
        let root_slot: NodeSlot = Rc::new(RefCell::new(Node::Leaf(leaf)));

        let bin = adw::Bin::builder().child(&root_widget).build();
        let state = Rc::new(RefCell::new(TreeState {
            root: root_slot,
            focused,
            manifests: Rc::new(manifests.to_vec()),
        }));

        Self { bin, state }
    }

    pub fn widget(&self) -> &adw::Bin {
        &self.bin
    }

    /// Split the currently focused leaf along `orientation`.
    ///
    /// The new leaf becomes focused.
    pub fn split(&self, orientation: Orientation) {
        let target = self.state.borrow().focused;
        let root_slot = self.state.borrow().root.clone();
        let manifests = self.state.borrow().manifests.clone();
        let bin = self.bin.clone();

        if let Some(new_focus) =
            walk_and_split(&root_slot, target, orientation, &bin, None, &manifests)
        {
            self.state.borrow_mut().focused = new_focus;
        }
    }

    /// Append a new terminal tab to the focused leaf.
    pub fn new_tab_in_focused(&self) {
        let target = self.state.borrow().focused;
        let root_slot = self.state.borrow().root.clone();
        let manifests = self.state.borrow().manifests.clone();
        let mut add = |leaf: &mut Leaf| leaf.add_tab(&manifests);
        with_leaf_mut(&root_slot, target, &mut add);
    }

    /// Focus the next leaf in depth-first order, wrapping around.
    pub fn focus_next_leaf(&self) {
        let leaves = self.collect_leaves();
        if leaves.len() < 2 {
            return;
        }
        let current = self.state.borrow().focused;
        let i = leaves.iter().position(|&id| id == current).unwrap_or(0);
        let next = leaves[(i + 1) % leaves.len()];
        self.set_focus(next);
    }

    /// Focus the previous leaf in depth-first order, wrapping around.
    pub fn focus_prev_leaf(&self) {
        let leaves = self.collect_leaves();
        if leaves.len() < 2 {
            return;
        }
        let current = self.state.borrow().focused;
        let i = leaves.iter().position(|&id| id == current).unwrap_or(0);
        let prev = if i == 0 { leaves.len() - 1 } else { i - 1 };
        self.set_focus(leaves[prev]);
    }

    fn set_focus(&self, target: LeafId) {
        let root_slot = self.state.borrow().root.clone();
        let mut grab = |leaf: &mut Leaf| {
            if let Some(idx) = leaf.notebook.current_page() {
                if let Some(page) = leaf.notebook.nth_page(Some(idx)) {
                    page.grab_focus();
                }
            }
        };
        if with_leaf_mut(&root_slot, target, &mut grab) {
            self.state.borrow_mut().focused = target;
        }
    }

    fn collect_leaves(&self) -> Vec<LeafId> {
        let mut out = Vec::new();
        collect_leaves_into(&self.state.borrow().root, &mut out);
        out
    }

    /// Walk the tree and return every [`TerminalPane`] currently mounted.
    /// Used by the [`crate::ipc::SocketService`] route to populate the
    /// pane registry whenever the active workspace changes.
    pub fn terminal_panes(&self) -> Vec<TerminalPane> {
        let mut out = Vec::new();
        collect_panes_into(&self.state.borrow().root, &mut out);
        out
    }

    /// Capture the tree's current structure for persistence. PTY contents
    /// are not part of the snapshot — only the leaf IDs, tab counts, and
    /// split shape.
    pub fn snapshot(&self) -> LayoutNode {
        snapshot_slot(&self.state.borrow().root)
    }

    /// Rebuild a tree from a snapshot. Each leaf gets fresh shells.
    pub fn from_snapshot(snapshot: &LayoutNode, manifests: &[CompiledManifest]) -> Self {
        let root_slot = build_from_snapshot(snapshot, manifests);
        let focused = first_leaf_id(&root_slot);
        let root_widget: Widget = match &*root_slot.borrow() {
            Node::Leaf(l) => l.notebook.clone().upcast(),
            Node::Split(s) => s.paned.clone().upcast(),
        };
        let bin = adw::Bin::builder().child(&root_widget).build();
        Self {
            bin,
            state: Rc::new(RefCell::new(TreeState {
                root: root_slot,
                focused,
                manifests: Rc::new(manifests.to_vec()),
            })),
        }
    }
}

fn snapshot_slot(slot: &NodeSlot) -> LayoutNode {
    match &*slot.borrow() {
        Node::Leaf(l) => LayoutNode::Leaf(LeafSnapshot {
            id: l.id,
            tabs: l.tabs.len().max(1),
        }),
        Node::Split(s) => LayoutNode::Split(SplitSnapshot {
            orientation: SnapshotOrientation::from_gtk(s.paned.orientation()),
            position: s.paned.position(),
            a: Box::new(snapshot_slot(&s.a)),
            b: Box::new(snapshot_slot(&s.b)),
        }),
    }
}

fn build_from_snapshot(node: &LayoutNode, manifests: &[CompiledManifest]) -> NodeSlot {
    match node {
        LayoutNode::Leaf(l) => {
            let leaf = Leaf::with_id_and_tabs(l.id, l.tabs, manifests);
            Rc::new(RefCell::new(Node::Leaf(leaf)))
        }
        LayoutNode::Split(s) => {
            let a_slot = build_from_snapshot(&s.a, manifests);
            let b_slot = build_from_snapshot(&s.b, manifests);
            let a_widget: Widget = match &*a_slot.borrow() {
                Node::Leaf(l) => l.notebook.clone().upcast(),
                Node::Split(s) => s.paned.clone().upcast(),
            };
            let b_widget: Widget = match &*b_slot.borrow() {
                Node::Leaf(l) => l.notebook.clone().upcast(),
                Node::Split(s) => s.paned.clone().upcast(),
            };
            let paned = Paned::builder()
                .orientation(s.orientation.to_gtk())
                .resize_start_child(true)
                .resize_end_child(true)
                .shrink_start_child(false)
                .shrink_end_child(false)
                .build();
            paned.set_start_child(Some(&a_widget));
            paned.set_end_child(Some(&b_widget));
            if s.position > 0 {
                paned.set_position(s.position);
            }
            Rc::new(RefCell::new(Node::Split(SplitNode {
                paned,
                a: a_slot,
                b: b_slot,
            })))
        }
    }
}

fn first_leaf_id(slot: &NodeSlot) -> Uuid {
    match &*slot.borrow() {
        Node::Leaf(l) => l.id,
        Node::Split(s) => first_leaf_id(&s.a),
    }
}

fn collect_leaves_into(slot: &NodeSlot, out: &mut Vec<LeafId>) {
    match &*slot.borrow() {
        Node::Leaf(l) => out.push(l.id),
        Node::Split(s) => {
            collect_leaves_into(&s.a, out);
            collect_leaves_into(&s.b, out);
        }
    }
}

fn collect_panes_into(slot: &NodeSlot, out: &mut Vec<TerminalPane>) {
    match &*slot.borrow() {
        Node::Leaf(l) => {
            for pane in &l.tabs {
                out.push(pane.clone());
            }
        }
        Node::Split(s) => {
            collect_panes_into(&s.a, out);
            collect_panes_into(&s.b, out);
        }
    }
}

fn with_leaf_mut<F>(slot: &NodeSlot, target: LeafId, f: &mut F) -> bool
where
    F: FnMut(&mut Leaf) + ?Sized,
{
    let kind = match &*slot.borrow() {
        Node::Leaf(l) if l.id == target => Step::Hit,
        Node::Leaf(_) => Step::Miss,
        Node::Split(s) => Step::Descend(s.a.clone(), s.b.clone()),
    };
    match kind {
        Step::Hit => {
            if let Node::Leaf(leaf) = &mut *slot.borrow_mut() {
                f(leaf);
            }
            true
        }
        Step::Miss => false,
        Step::Descend(a, b) => with_leaf_mut(&a, target, f) || with_leaf_mut(&b, target, f),
    }
}

enum Step {
    Hit,
    Miss,
    Descend(NodeSlot, NodeSlot),
}

fn walk_and_split(
    slot: &NodeSlot,
    target: LeafId,
    orientation: Orientation,
    root_bin: &adw::Bin,
    parent: Option<(Paned, ChildSlot)>,
    manifests: &[CompiledManifest],
) -> Option<LeafId> {
    // Probe what kind of node we're looking at without holding the borrow during recursion.
    let action = match &*slot.borrow() {
        Node::Leaf(l) if l.id == target => Action::SplitHere,
        Node::Leaf(_) => Action::None,
        Node::Split(s) => Action::Descend {
            paned: s.paned.clone(),
            a: s.a.clone(),
            b: s.b.clone(),
        },
    };
    match action {
        Action::None => None,
        Action::Descend { paned, a, b } => walk_and_split(
            &a,
            target,
            orientation,
            root_bin,
            Some((paned.clone(), ChildSlot::Start)),
            manifests,
        )
        .or_else(|| {
            walk_and_split(
                &b,
                target,
                orientation,
                root_bin,
                Some((paned, ChildSlot::End)),
                manifests,
            )
        }),
        Action::SplitHere => {
            // Build the new structure outside the borrow.
            let new_leaf = Leaf::new(manifests);
            let new_leaf_id = new_leaf.id;
            let paned = Paned::builder()
                .orientation(orientation)
                .resize_start_child(true)
                .resize_end_child(true)
                .shrink_start_child(false)
                .shrink_end_child(false)
                .build();

            // Snapshot the old leaf's notebook widget (we'll re-parent it into the new Paned).
            let old_widget: Widget = match &*slot.borrow() {
                Node::Leaf(l) => l.notebook.clone().upcast(),
                _ => unreachable!(),
            };

            // Detach old_widget from its current parent before attaching elsewhere.
            detach_from_parent(&old_widget);

            paned.set_start_child(Some(&old_widget));
            paned.set_end_child(Some(&new_leaf.notebook));

            // Replace the slot's Node: Leaf -> Split, keeping the old leaf inside as `a`.
            let old_node = std::mem::replace(
                &mut *slot.borrow_mut(),
                Node::Split(SplitNode {
                    paned: paned.clone(),
                    a: Rc::new(RefCell::new(Node::Leaf(Leaf {
                        // placeholder, replaced below
                        id: Uuid::nil(),
                        notebook: Notebook::new(),
                        tabs: Vec::new(),
                    }))),
                    b: Rc::new(RefCell::new(Node::Leaf(new_leaf))),
                }),
            );
            // Fix up `a` to hold the real old leaf.
            if let Node::Split(s) = &mut *slot.borrow_mut() {
                *s.a.borrow_mut() = old_node;
            }

            // Update the parent container's child reference: it used to point at the old
            // widget; now it should point at the new paned.
            match parent {
                Some((p, ChildSlot::Start)) => p.set_start_child(Some(&paned)),
                Some((p, ChildSlot::End)) => p.set_end_child(Some(&paned)),
                None => root_bin.set_child(Some(&paned)),
            }

            Some(new_leaf_id)
        }
    }
}

enum Action {
    None,
    SplitHere,
    Descend {
        paned: Paned,
        a: NodeSlot,
        b: NodeSlot,
    },
}

fn detach_from_parent(widget: &Widget) {
    let Some(parent) = widget.parent() else {
        return;
    };
    if let Some(bin) = parent.downcast_ref::<adw::Bin>() {
        bin.set_child(Widget::NONE);
    } else if let Some(p) = parent.downcast_ref::<Paned>() {
        if p.start_child().as_ref() == Some(widget) {
            p.set_start_child(Widget::NONE);
        } else if p.end_child().as_ref() == Some(widget) {
            p.set_end_child(Widget::NONE);
        }
    } else if let Some(nb) = parent.downcast_ref::<Notebook>() {
        if let Some(idx) = nb.page_num(widget) {
            nb.remove_page(Some(idx));
        }
    }
}
