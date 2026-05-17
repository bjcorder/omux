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
use std::rc::{Rc, Weak};

use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{Notebook, Orientation, Paned, Widget};
use libadwaita as adw;
use libadwaita::prelude::*;
use uuid::Uuid;

use super::Pane;
use super::browser::BrowserPane;
use super::terminal::TerminalPane;
use crate::agent::manifest::CompiledManifest;
use crate::workspace::SnapshotOrientation;
use crate::workspace::snapshot::{LayoutNode, LeafSnapshot, SplitSnapshot, TabKind, TabSnapshot};
use webkit6::NetworkSession;

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
    network_session: NetworkSession,
}

type NodeSlot = Rc<RefCell<Node>>;

enum Node {
    Leaf(Leaf),
    Split(SplitNode),
}

struct Leaf {
    id: LeafId,
    notebook: Notebook,
    tabs: Vec<Pane>,
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
    fn new(manifests: &[CompiledManifest], session: &NetworkSession) -> Self {
        Self::with_id_and_tabs(
            Uuid::new_v4(),
            &[TabSnapshot::terminal()],
            manifests,
            session,
        )
    }

    fn with_id_and_tabs(
        id: Uuid,
        tabs: &[TabSnapshot],
        manifests: &[CompiledManifest],
        session: &NetworkSession,
    ) -> Self {
        let notebook = Notebook::builder()
            .scrollable(true)
            .show_border(false)
            .build();
        let mut leaf = Self {
            id,
            notebook,
            tabs: Vec::new(),
        };
        let specs = if tabs.is_empty() {
            &[TabSnapshot::terminal()][..]
        } else {
            tabs
        };
        for spec in specs {
            leaf.add_tab_from_spec(spec, manifests, session);
        }
        leaf
    }

    fn add_terminal_tab(&mut self, manifests: &[CompiledManifest]) {
        let pane = Pane::Terminal(TerminalPane::new_with_manifests(manifests));
        self.append_pane(pane);
    }

    fn add_browser_tab(&mut self, session: &NetworkSession, url: Option<&str>) {
        let pane = Pane::Browser(BrowserPane::new(session, url));
        self.append_pane(pane);
    }

    fn add_tab_from_spec(
        &mut self,
        spec: &TabSnapshot,
        manifests: &[CompiledManifest],
        session: &NetworkSession,
    ) {
        match spec.kind {
            TabKind::Terminal => self.add_terminal_tab(manifests),
            TabKind::Browser => self.add_browser_tab(session, spec.url.as_deref()),
        }
    }

    fn append_pane(&mut self, pane: Pane) {
        let label = gtk4::Label::new(Some(pane.tab_label()));
        let idx = self.notebook.append_page(pane.widget(), Some(&label));
        self.notebook.set_current_page(Some(idx));
        pane.grab_inner_focus();
        self.tabs.push(pane);
    }

    fn snapshot_tabs(&self) -> Vec<TabSnapshot> {
        self.tabs
            .iter()
            .map(|p| match p {
                Pane::Terminal(_) => TabSnapshot::terminal(),
                Pane::Browser(b) => TabSnapshot::browser(b.current_url()),
            })
            .collect()
    }
}

impl PaneTree {
    /// Build an empty tree with a single leaf containing one tab. Use
    /// [`PaneTree::from_snapshot`] for the production path (it handles
    /// the single-leaf case via [`LayoutNode::single_leaf`]).
    #[allow(dead_code)]
    pub fn new(manifests: &[CompiledManifest], network_session: NetworkSession) -> Self {
        let leaf = Leaf::new(manifests, &network_session);
        let focused = leaf.id;
        let root_widget: Widget = leaf.notebook.clone().upcast();
        let root_slot: NodeSlot = Rc::new(RefCell::new(Node::Leaf(leaf)));

        let bin = adw::Bin::builder().child(&root_widget).build();
        let state = Rc::new(RefCell::new(TreeState {
            root: root_slot,
            focused,
            manifests: Rc::new(manifests.to_vec()),
            network_session,
        }));

        let me = Self { bin, state };
        me.install_focus_in_subtree(&me.state.borrow().root);
        me
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
        let session = self.state.borrow().network_session.clone();
        let bin = self.bin.clone();

        if let Some(new_focus) = walk_and_split(
            &root_slot,
            target,
            orientation,
            &bin,
            None,
            &manifests,
            &session,
        ) {
            self.state.borrow_mut().focused = new_focus;
            // Install focus tracking on the panes of the newly created leaf.
            let state_weak = Rc::downgrade(&self.state);
            let new_slot = find_leaf_slot(&self.state.borrow().root, new_focus);
            if let Some(slot) = new_slot {
                install_focus_in_slot(&slot, &state_weak);
            }
        }
    }

    /// Append a new terminal tab to the focused leaf.
    pub fn new_tab_in_focused(&self) {
        let target = self.state.borrow().focused;
        let root_slot = self.state.borrow().root.clone();
        let manifests = self.state.borrow().manifests.clone();
        let state_weak = Rc::downgrade(&self.state);
        let mut add = |leaf: &mut Leaf| {
            leaf.add_terminal_tab(&manifests);
            if let Some(p) = leaf.tabs.last() {
                install_focus_tracking(p, leaf.id, &state_weak);
            }
        };
        with_leaf_mut(&root_slot, target, &mut add);
    }

    /// Append a new browser tab to the focused leaf.
    pub fn new_browser_tab_in_focused(&self, url: Option<&str>) {
        let target = self.state.borrow().focused;
        let root_slot = self.state.borrow().root.clone();
        let session = self.state.borrow().network_session.clone();
        let state_weak = Rc::downgrade(&self.state);
        let mut add = |leaf: &mut Leaf| {
            leaf.add_browser_tab(&session, url);
            if let Some(p) = leaf.tabs.last() {
                install_focus_tracking(p, leaf.id, &state_weak);
            }
        };
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
    /// pane registry whenever the active workspace changes. Browser panes
    /// are intentionally skipped — they never produce hook events.
    pub fn terminal_panes(&self) -> Vec<TerminalPane> {
        let mut out = Vec::new();
        collect_terminal_panes_into(&self.state.borrow().root, &mut out);
        out
    }

    /// Capture the tree's current structure for persistence. PTY contents
    /// are not part of the snapshot — only the leaf IDs, per-tab kinds (+
    /// URL for browser tabs), and split shape.
    pub fn snapshot(&self) -> LayoutNode {
        snapshot_slot(&self.state.borrow().root)
    }

    /// Rebuild a tree from a snapshot. Each leaf gets fresh shells / new
    /// browser views loading the saved URL.
    pub fn from_snapshot(
        snapshot: &LayoutNode,
        manifests: &[CompiledManifest],
        network_session: NetworkSession,
    ) -> Self {
        let root_slot = build_from_snapshot(snapshot, manifests, &network_session);
        let focused = first_leaf_id(&root_slot);
        let root_widget: Widget = match &*root_slot.borrow() {
            Node::Leaf(l) => l.notebook.clone().upcast(),
            Node::Split(s) => s.paned.clone().upcast(),
        };
        let bin = adw::Bin::builder().child(&root_widget).build();
        let me = Self {
            bin,
            state: Rc::new(RefCell::new(TreeState {
                root: root_slot,
                focused,
                manifests: Rc::new(manifests.to_vec()),
                network_session,
            })),
        };
        me.install_focus_in_subtree(&me.state.borrow().root);
        me
    }

    /// Walk the slot's subtree and attach an `EventControllerFocus` to
    /// every pane's outer frame so focus → `TreeState::focused` updates
    /// automatically when the user clicks into a pane.
    fn install_focus_in_subtree(&self, slot: &NodeSlot) {
        let state_weak = Rc::downgrade(&self.state);
        install_focus_in_slot(slot, &state_weak);
    }
}

fn install_focus_in_slot(slot: &NodeSlot, state_weak: &Weak<RefCell<TreeState>>) {
    match &*slot.borrow() {
        Node::Leaf(l) => {
            for pane in &l.tabs {
                install_focus_tracking(pane, l.id, state_weak);
            }
        }
        Node::Split(s) => {
            install_focus_in_slot(&s.a, state_weak);
            install_focus_in_slot(&s.b, state_weak);
        }
    }
}

fn install_focus_tracking(pane: &Pane, leaf_id: LeafId, state_weak: &Weak<RefCell<TreeState>>) {
    let controller = gtk::EventControllerFocus::new();
    let state_weak = state_weak.clone();
    controller.connect_contains_focus_notify(move |c| {
        if c.contains_focus()
            && let Some(state) = state_weak.upgrade()
        {
            state.borrow_mut().focused = leaf_id;
        }
    });
    pane.widget().add_controller(controller);
}

fn snapshot_slot(slot: &NodeSlot) -> LayoutNode {
    match &*slot.borrow() {
        Node::Leaf(l) => LayoutNode::Leaf(LeafSnapshot {
            id: l.id,
            tabs: l.snapshot_tabs(),
        }),
        Node::Split(s) => LayoutNode::Split(SplitSnapshot {
            orientation: SnapshotOrientation::from_gtk(s.paned.orientation()),
            position: s.paned.position(),
            a: Box::new(snapshot_slot(&s.a)),
            b: Box::new(snapshot_slot(&s.b)),
        }),
    }
}

fn build_from_snapshot(
    node: &LayoutNode,
    manifests: &[CompiledManifest],
    session: &NetworkSession,
) -> NodeSlot {
    match node {
        LayoutNode::Leaf(l) => {
            let leaf = Leaf::with_id_and_tabs(l.id, &l.tabs, manifests, session);
            Rc::new(RefCell::new(Node::Leaf(leaf)))
        }
        LayoutNode::Split(s) => {
            let a_slot = build_from_snapshot(&s.a, manifests, session);
            let b_slot = build_from_snapshot(&s.b, manifests, session);
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

/// Find the [`NodeSlot`] of a specific leaf, returning a clone of the
/// `Rc<RefCell<Node>>` that holds it.
fn find_leaf_slot(slot: &NodeSlot, target: LeafId) -> Option<NodeSlot> {
    let kind = match &*slot.borrow() {
        Node::Leaf(l) if l.id == target => return Some(slot.clone()),
        Node::Leaf(_) => return None,
        Node::Split(s) => (s.a.clone(), s.b.clone()),
    };
    find_leaf_slot(&kind.0, target).or_else(|| find_leaf_slot(&kind.1, target))
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

fn collect_terminal_panes_into(slot: &NodeSlot, out: &mut Vec<TerminalPane>) {
    match &*slot.borrow() {
        Node::Leaf(l) => {
            for pane in &l.tabs {
                if let Some(t) = pane.as_terminal() {
                    out.push(t.clone());
                }
            }
        }
        Node::Split(s) => {
            collect_terminal_panes_into(&s.a, out);
            collect_terminal_panes_into(&s.b, out);
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
    session: &NetworkSession,
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
            session,
        )
        .or_else(|| {
            walk_and_split(
                &b,
                target,
                orientation,
                root_bin,
                Some((paned, ChildSlot::End)),
                manifests,
                session,
            )
        }),
        Action::SplitHere => {
            // Build the new structure outside the borrow.
            let new_leaf = Leaf::new(manifests, session);
            let new_leaf_id = new_leaf.id;
            // Install focus tracking on the new leaf's panes. The old
            // leaf already has controllers from initial construction.
            // We defer access to TreeState — the slot's NodeSlot doesn't
            // carry a back-reference. This is handled at the caller in
            // PaneTree::split via post_split_install_focus below.
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
