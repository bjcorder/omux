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
    fn new() -> Self {
        let notebook = Notebook::builder()
            .scrollable(true)
            .show_border(false)
            .build();
        let mut leaf = Self {
            id: Uuid::new_v4(),
            notebook,
            tabs: Vec::new(),
        };
        leaf.add_tab();
        leaf
    }

    fn add_tab(&mut self) {
        let pane = TerminalPane::new();
        let label = gtk4::Label::new(Some("shell"));
        let idx = self.notebook.append_page(pane.widget(), Some(&label));
        self.notebook.set_current_page(Some(idx));
        pane.widget().grab_focus();
        self.tabs.push(pane);
    }
}

impl PaneTree {
    pub fn new() -> Self {
        let leaf = Leaf::new();
        let focused = leaf.id;
        let root_widget: Widget = leaf.notebook.clone().upcast();
        let root_slot: NodeSlot = Rc::new(RefCell::new(Node::Leaf(leaf)));

        let bin = adw::Bin::builder().child(&root_widget).build();
        let state = Rc::new(RefCell::new(TreeState {
            root: root_slot,
            focused,
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
        let bin = self.bin.clone();

        if let Some(new_focus) = walk_and_split(&root_slot, target, orientation, &bin, None) {
            self.state.borrow_mut().focused = new_focus;
        }
    }

    /// Append a new terminal tab to the focused leaf.
    pub fn new_tab_in_focused(&self) {
        let target = self.state.borrow().focused;
        let root_slot = self.state.borrow().root.clone();
        let mut add = |leaf: &mut Leaf| leaf.add_tab();
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
        )
        .or_else(|| {
            walk_and_split(
                &b,
                target,
                orientation,
                root_bin,
                Some((paned, ChildSlot::End)),
            )
        }),
        Action::SplitHere => {
            // Build the new structure outside the borrow.
            let new_leaf = Leaf::new();
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
