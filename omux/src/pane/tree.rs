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
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{Notebook, Orientation, Paned, Widget};
use libadwaita as adw;
use libadwaita::prelude::*;
use uuid::Uuid;

use super::Pane;
use super::browser::BrowserPane;
use super::terminal::TerminalPane;
use crate::agent::manifest::CompiledManifest;
use crate::agent::status::PaneStatus;
use crate::workspace::SnapshotOrientation;
use crate::workspace::snapshot::{LayoutNode, LeafSnapshot, SplitSnapshot, TabKind, TabSnapshot};
use webkit6::NetworkSession;

/// Attach a `+` MenuButton to the right side of the Notebook's tab
/// bar. Clicking it opens a popover with "New terminal tab" / "New
/// browser tab" entries. The actions live on the `leaf.*` action group
/// installed per-leaf by [`install_focus_in_slot`].
fn attach_new_tab_button(notebook: &Notebook) {
    let menu = gio::Menu::new();
    menu.append(Some("New terminal tab"), Some("leaf.new-terminal"));
    menu.append(Some("New browser tab"), Some("leaf.new-browser"));

    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    let btn = gtk::MenuButton::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add tab")
        .popover(&popover)
        .css_classes(["flat", "leaf-add-tab"])
        .build();

    notebook.set_action_widget(&btn, gtk::PackType::End);
}

/// Build the per-tab label widget:
///
/// ```text
///   [ ! ] shell  [ × ]
///    ^badge       ^close
/// ```
///
/// The badge is hidden until the pane needs attention. The close button
/// fires `clicked` events but has no handler attached here — the
/// post-construction install walk wires it up so the handler can capture
/// a Weak<RefCell<TreeState>> for removal.
fn make_tab_label(text: &str) -> (gtk::Box, gtk::Image, gtk::Button) {
    let badge = gtk::Image::from_icon_name("emblem-important-symbolic");
    badge.set_pixel_size(10);
    badge.add_css_class("tab-needs-attention");
    badge.set_tooltip_text(Some("Agent needs attention"));
    badge.set_visible(false);

    let label = gtk::Label::new(Some(text));

    let close_btn = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .tooltip_text("Close tab")
        .css_classes(["flat", "tab-close"])
        .build();
    close_btn.set_can_focus(false);

    let hbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(4)
        .build();
    hbox.append(&badge);
    hbox.append(&label);
    hbox.append(&close_btn);
    (hbox, badge, close_btn)
}

pub type LeafId = Uuid;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChildSlot {
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// One ancestor of a leaf, recorded by [`path_to_leaf`]. Ordered
/// nearest-ancestor-first.
struct PathStep {
    split_slot: NodeSlot,
    came_from: ChildSlot,
    orientation: gtk::Orientation,
}

impl PathStep {
    fn came_from_matches(&self, side: ChildSlot) -> bool {
        self.came_from == side
    }
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
    /// Top-level container the tree mounts into. Kept here so structural
    /// mutations (`collapse_leaf`) can re-parent the surviving subtree
    /// when the closed leaf was a direct child of root.
    bin: adw::Bin,
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
    /// One `tab-badge` Image per tab, parallel to `tabs`. Hidden while
    /// the tab's pane is in any state other than `NeedsAttention`.
    tab_badges: Vec<gtk::Image>,
    /// One close (`×`) Button per tab, parallel to `tabs`. The button's
    /// `clicked` handler is wired during the post-construction install
    /// walk (where the tree state Weak ref is available).
    tab_close_buttons: Vec<gtk::Button>,
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
        attach_new_tab_button(&notebook);
        let mut leaf = Self {
            id,
            notebook,
            tabs: Vec::new(),
            tab_badges: Vec::new(),
            tab_close_buttons: Vec::new(),
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
        let (tab_widget, badge, close_btn) = make_tab_label(pane.tab_label());
        let idx = self.notebook.append_page(pane.widget(), Some(&tab_widget));
        self.notebook.set_current_page(Some(idx));
        pane.grab_inner_focus();
        self.tabs.push(pane);
        self.tab_badges.push(badge);
        self.tab_close_buttons.push(close_btn);
    }

    /// Close the currently selected tab. Returns `(closed, became_empty)`.
    /// Caller decides whether to collapse the leaf or refuse.
    #[allow(dead_code)] // Reserved for future close-current paths that take a leaf reference.
    fn close_current_tab(&mut self) -> (bool, bool) {
        let Some(idx) = self.notebook.current_page() else {
            return (false, false);
        };
        self.close_tab_at(idx as usize)
    }

    /// Close the tab at the given index. Returns `(closed, became_empty)`.
    fn close_tab_at(&mut self, idx: usize) -> (bool, bool) {
        if idx >= self.tabs.len() {
            return (false, false);
        }
        self.notebook.remove_page(Some(idx as u32));
        self.tabs.remove(idx);
        self.tab_badges.remove(idx);
        self.tab_close_buttons.remove(idx);
        (true, self.tabs.is_empty())
    }

    /// Sync each tab's badge visibility to its pane's current status.
    fn refresh_tab_badges(&self) {
        for (pane, badge) in self.tabs.iter().zip(self.tab_badges.iter()) {
            let want_visible = match pane {
                Pane::Terminal(t) => t.status() == PaneStatus::NeedsAttention,
                Pane::Browser(_) => false,
            };
            if badge.is_visible() != want_visible {
                badge.set_visible(want_visible);
            }
        }
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
            bin: bin.clone(),
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
            wire_last_pane(leaf, &state_weak);
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
            wire_last_pane(leaf, &state_weak);
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

    /// Focus the adjacent leaf in a cardinal direction (tmux-style
    /// `Alt+Arrow` nav). Walks up the tree from the current leaf until
    /// it finds a split with the matching orientation + descent direction,
    /// then descends extreme-side into the sibling.
    pub fn focus_in_direction(&self, dir: Direction) {
        let current = self.state.borrow().focused;
        let root = self.state.borrow().root.clone();
        let Some(path) = path_to_leaf(&root, current) else {
            return;
        };
        let (want_orientation, want_came_from, descend_extreme) = match dir {
            Direction::Right => (
                gtk::Orientation::Horizontal,
                ChildSlot::Start,
                ChildSlot::Start,
            ),
            Direction::Left => (gtk::Orientation::Horizontal, ChildSlot::End, ChildSlot::End),
            Direction::Down => (
                gtk::Orientation::Vertical,
                ChildSlot::Start,
                ChildSlot::Start,
            ),
            Direction::Up => (gtk::Orientation::Vertical, ChildSlot::End, ChildSlot::End),
        };
        for step in &path {
            if step.orientation == want_orientation && step.came_from_matches(want_came_from) {
                // Switch to the other child, then descend extreme-side to a leaf.
                let Node::Split(s) = &*step.split_slot.borrow() else {
                    continue;
                };
                let sibling = match want_came_from {
                    ChildSlot::Start => s.b.clone(),
                    ChildSlot::End => s.a.clone(),
                };
                if let Some(target) = extreme_leaf_id(&sibling, descend_extreme) {
                    self.set_focus(target);
                }
                return;
            }
        }
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

    /// Walk every leaf and ensure each tab's badge reflects the current
    /// status of its pane. Called from a polling timer in the shell.
    pub fn refresh_badges(&self) {
        refresh_badges_in_slot(&self.state.borrow().root);
    }

    /// Close the focused leaf's currently active tab. If that was the
    /// last tab AND the leaf is part of a split, the now-empty leaf
    /// collapses out of the tree (the sibling subtree takes its place).
    /// If the leaf is the root of the workspace, the last tab refuses
    /// to close — we don't allow an empty workspace.
    pub fn close_focused_tab(&self) -> bool {
        let target = self.state.borrow().focused;
        let root_slot = self.state.borrow().root.clone();
        // path_is_root walks the tree borrowing slots — compute BEFORE
        // with_leaf_mut grabs borrow_mut on the target leaf, otherwise
        // the path walk re-borrows the same slot and panics.
        let is_root_leaf = path_is_root(&root_slot, target);
        let mut closed = false;
        let mut became_empty = false;
        let mut close = |leaf: &mut Leaf| {
            let Some(i) = leaf.notebook.current_page() else {
                return;
            };
            if is_root_leaf && leaf.tabs.len() <= 1 {
                return;
            }
            let (c, e) = leaf.close_tab_at(i as usize);
            closed = c;
            became_empty = e;
        };
        with_leaf_mut(&root_slot, target, &mut close);
        if closed && became_empty {
            collapse_leaf(&self.state, target);
        }
        closed
    }

    /// Copy the active terminal's selection.
    pub fn copy_active_selection(&self) {
        if let Some(pane) = self.focused_active_pane()
            && let Pane::Terminal(t) = &pane
        {
            t.copy_selection();
        }
    }

    /// Paste into the active terminal.
    pub fn paste_to_active(&self) {
        if let Some(pane) = self.focused_active_pane()
            && let Pane::Terminal(t) = &pane
        {
            t.paste_clipboard();
        }
    }

    fn focused_active_pane(&self) -> Option<Pane> {
        let target = self.state.borrow().focused;
        let root_slot = self.state.borrow().root.clone();
        let mut out: Option<Pane> = None;
        let mut grab = |leaf: &mut Leaf| {
            if let Some(idx) = leaf.notebook.current_page() {
                if let Some(p) = leaf.tabs.get(idx as usize) {
                    out = Some(p.clone());
                }
            }
        };
        with_leaf_mut(&root_slot, target, &mut grab);
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
            bin: bin.clone(),
            state: Rc::new(RefCell::new(TreeState {
                root: root_slot,
                focused,
                manifests: Rc::new(manifests.to_vec()),
                network_session,
                bin,
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
            install_leaf_actions(l, state_weak);
            install_close_buttons(l, state_weak);
        }
        Node::Split(s) => {
            install_focus_in_slot(&s.a, state_weak);
            install_focus_in_slot(&s.b, state_weak);
        }
    }
}

/// Wire each tab's `×` close button to remove its pane.
///
/// Idempotency note: this is only ever called by [`install_focus_in_slot`]
/// once per leaf — either at `PaneTree::from_snapshot` / `PaneTree::new`
/// time, or for a freshly-created new leaf after a split. Buttons added
/// later (via `new_tab_in_focused`, `new_browser_tab_in_focused`, or the
/// per-leaf `+` actions) are wired in [`wire_last_pane`]. So we never
/// re-walk an already-wired leaf and don't need a guard.
fn install_close_buttons(leaf: &Leaf, state_weak: &Weak<RefCell<TreeState>>) {
    let leaf_id = leaf.id;
    for (pane, button) in leaf.tabs.iter().zip(leaf.tab_close_buttons.iter()) {
        let pane_id = pane.pane_id();
        let state_weak = state_weak.clone();
        button.connect_clicked(move |_| {
            tracing::debug!(leaf = %leaf_id, pane = %pane_id, "tab close button clicked");
            if let Some(state) = state_weak.upgrade() {
                close_tab_by_pane(&state, leaf_id, pane_id);
            }
        });
    }
}

fn close_tab_by_pane(state: &Rc<RefCell<TreeState>>, leaf_id: LeafId, pane_id: Uuid) {
    let root_slot = state.borrow().root.clone();
    // See close_focused_tab: compute outside the with_leaf_mut borrow.
    let is_root_leaf = path_is_root(&root_slot, leaf_id);

    let mut closed = false;
    let mut became_empty = false;
    let mut close = |leaf: &mut Leaf| {
        let Some(idx) = leaf.tabs.iter().position(|p| p.pane_id() == pane_id) else {
            return;
        };
        if is_root_leaf && leaf.tabs.len() <= 1 {
            return;
        }
        let (c, e) = leaf.close_tab_at(idx);
        closed = c;
        became_empty = e;
    };
    with_leaf_mut(&root_slot, leaf_id, &mut close);

    if closed && became_empty {
        collapse_leaf(state, leaf_id);
    }
}

/// True iff the given leaf id is the root node of the tree (i.e., has
/// no parent split). Used to refuse closing the last tab of a workspace
/// (we don't allow empty workspaces — only empty leaves inside splits
/// get collapsed away).
fn path_is_root(root_slot: &NodeSlot, leaf_id: LeafId) -> bool {
    path_to_leaf(root_slot, leaf_id)
        .map(|p| p.is_empty())
        .unwrap_or(false)
}

/// Collapse a now-empty leaf out of its parent split: replace the
/// parent split with the surviving sibling subtree, and re-parent the
/// sibling widget into the grandparent (or the root bin if the parent
/// was the root). A no-op if the leaf has no parent — refuse-policy
/// for root leaves is enforced by callers before invoking this.
fn collapse_leaf(state: &Rc<RefCell<TreeState>>, target: LeafId) {
    let root_slot = state.borrow().root.clone();
    let bin = state.borrow().bin.clone();

    let path = match path_to_leaf(&root_slot, target) {
        Some(p) if !p.is_empty() => p,
        _ => return,
    };
    let parent_step = &path[0];

    // Identify the sibling slot under the parent split.
    let sibling_slot = {
        let parent = parent_step.split_slot.borrow();
        let Node::Split(s) = &*parent else { return };
        match parent_step.came_from {
            ChildSlot::Start => s.b.clone(),
            ChildSlot::End => s.a.clone(),
        }
    };

    // Grab the sibling's current widget (we'll re-parent it).
    let sibling_widget: Widget = match &*sibling_slot.borrow() {
        Node::Leaf(l) => l.notebook.clone().upcast(),
        Node::Split(s) => s.paned.clone().upcast(),
    };
    detach_from_parent(&sibling_widget);

    // Move sibling's Node into the parent's slot. Leave a throwaway
    // Leaf in the sibling slot so RefCell is still valid; it'll be
    // dropped when the Rc goes out of scope.
    let throwaway = Node::Leaf(Leaf {
        id: Uuid::nil(),
        notebook: Notebook::new(),
        tabs: Vec::new(),
        tab_badges: Vec::new(),
        tab_close_buttons: Vec::new(),
    });
    let sibling_node = std::mem::replace(&mut *sibling_slot.borrow_mut(), throwaway);
    *parent_step.split_slot.borrow_mut() = sibling_node;

    // Wire the surviving subtree into the grandparent (or root bin).
    match path.get(1) {
        Some(grandparent) => {
            let gp_borrow = grandparent.split_slot.borrow();
            if let Node::Split(s) = &*gp_borrow {
                match grandparent.came_from {
                    ChildSlot::Start => s.paned.set_start_child(Some(&sibling_widget)),
                    ChildSlot::End => s.paned.set_end_child(Some(&sibling_widget)),
                }
            }
        }
        None => {
            // Parent was the tree root.
            bin.set_child(Some(&sibling_widget));
        }
    }

    // Move focus into a leaf that actually exists in the surviving tree.
    let new_focus = first_leaf_id(&state.borrow().root);
    state.borrow_mut().focused = new_focus;
    tracing::debug!(closed = %target, new_focus = %new_focus, "collapsed empty leaf");
}

/// Install the `leaf.new-terminal` and `leaf.new-browser` actions on
/// this leaf's notebook. The popover menu attached to the leaf's
/// `+` MenuButton references these via `leaf.*` so each leaf's button
/// adds tabs to *itself*, not to whichever leaf is currently focused.
fn install_leaf_actions(leaf: &Leaf, state_weak: &Weak<RefCell<TreeState>>) {
    let actions = gio::SimpleActionGroup::new();
    let leaf_id = leaf.id;

    let state_for_term = state_weak.clone();
    let new_term = gio::SimpleAction::new("new-terminal", None);
    new_term.connect_activate(move |_, _| {
        if let Some(state) = state_for_term.upgrade() {
            add_terminal_tab_in_leaf(&state, leaf_id);
        }
    });
    actions.add_action(&new_term);

    let state_for_browser = state_weak.clone();
    let new_browser = gio::SimpleAction::new("new-browser", None);
    new_browser.connect_activate(move |_, _| {
        if let Some(state) = state_for_browser.upgrade() {
            add_browser_tab_in_leaf(&state, leaf_id);
        }
    });
    actions.add_action(&new_browser);

    leaf.notebook.insert_action_group("leaf", Some(&actions));
}

fn add_terminal_tab_in_leaf(state: &Rc<RefCell<TreeState>>, target: LeafId) {
    let root_slot = state.borrow().root.clone();
    let manifests = state.borrow().manifests.clone();
    let state_weak = Rc::downgrade(state);
    let mut add = |leaf: &mut Leaf| {
        leaf.add_terminal_tab(&manifests);
        wire_last_pane(leaf, &state_weak);
    };
    with_leaf_mut(&root_slot, target, &mut add);
}

fn add_browser_tab_in_leaf(state: &Rc<RefCell<TreeState>>, target: LeafId) {
    let root_slot = state.borrow().root.clone();
    let session = state.borrow().network_session.clone();
    let state_weak = Rc::downgrade(state);
    let mut add = |leaf: &mut Leaf| {
        leaf.add_browser_tab(&session, None);
        wire_last_pane(leaf, &state_weak);
    };
    with_leaf_mut(&root_slot, target, &mut add);
}

/// Helper called after `add_terminal_tab` / `add_browser_tab` to attach
/// focus tracking AND the close button click handler to the just-added
/// pane. The leaf-level action group (for the `+` button) is per-leaf
/// and was installed once at construction time, so it doesn't need
/// re-installing here.
fn wire_last_pane(leaf: &Leaf, state_weak: &Weak<RefCell<TreeState>>) {
    let Some(p) = leaf.tabs.last() else { return };
    install_focus_tracking(p, leaf.id, state_weak);
    if let Some(btn) = leaf.tab_close_buttons.last() {
        let leaf_id = leaf.id;
        let pane_id = p.pane_id();
        let state_weak = state_weak.clone();
        btn.connect_clicked(move |_| {
            if let Some(state) = state_weak.upgrade() {
                close_tab_by_pane(&state, leaf_id, pane_id);
            }
        });
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

/// Descend the subtree at `slot` always to the `side` child, returning
/// the LeafId we land on. `side == Start` gives the topmost/leftmost
/// leaf, `End` gives the bottommost/rightmost.
fn extreme_leaf_id(slot: &NodeSlot, side: ChildSlot) -> Option<Uuid> {
    match &*slot.borrow() {
        Node::Leaf(l) => Some(l.id),
        Node::Split(s) => match side {
            ChildSlot::Start => extreme_leaf_id(&s.a, side),
            ChildSlot::End => extreme_leaf_id(&s.b, side),
        },
    }
}

/// Build the ancestor chain from the given leaf back up to the root.
/// Returned vec is ordered closest-ancestor first.
fn path_to_leaf(slot: &NodeSlot, target: LeafId) -> Option<Vec<PathStep>> {
    let probe = match &*slot.borrow() {
        Node::Leaf(l) if l.id == target => return Some(Vec::new()),
        Node::Leaf(_) => return None,
        Node::Split(s) => (s.a.clone(), s.b.clone(), s.paned.orientation()),
    };
    let (a, b, orientation) = probe;
    if let Some(mut path) = path_to_leaf(&a, target) {
        path.push(PathStep {
            split_slot: slot.clone(),
            came_from: ChildSlot::Start,
            orientation,
        });
        return Some(path);
    }
    if let Some(mut path) = path_to_leaf(&b, target) {
        path.push(PathStep {
            split_slot: slot.clone(),
            came_from: ChildSlot::End,
            orientation,
        });
        return Some(path);
    }
    None
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

fn refresh_badges_in_slot(slot: &NodeSlot) {
    match &*slot.borrow() {
        Node::Leaf(l) => l.refresh_tab_badges(),
        Node::Split(s) => {
            refresh_badges_in_slot(&s.a);
            refresh_badges_in_slot(&s.b);
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
                        tab_badges: Vec::new(),
                        tab_close_buttons: Vec::new(),
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
