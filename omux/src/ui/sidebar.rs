//! Workspaces sidebar.
//!
//! A vertical `gtk::Box` containing a scrolling `gtk::ListBox` of
//! workspace rows and a "New workspace" button. Each row is a
//! `gtk::ListBoxRow` carrying the workspace name as data; pinned
//! workspaces show a small pin icon to the left of the name.
//!
//! Callers wire callbacks through the setters (`on_select`, `on_new`,
//! `on_rename`, `on_delete`, `on_pin_toggle`, `on_reorder`). The widget
//! does not own any persistence state; it only mirrors a list the shell
//! passes in via [`Sidebar::set_workspaces`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;

pub struct WorkspaceRowData {
    pub name: String,
    pub pinned: bool,
}

type StringFn = Box<dyn Fn(&str)>;
type StringStringFn = Box<dyn Fn(&str, &str)>;
type EmptyFn = Box<dyn Fn()>;
type ReorderListFn = Box<dyn Fn(Vec<String>)>;

#[derive(Default)]
struct Callbacks {
    on_select: Option<StringFn>,
    on_new: Option<EmptyFn>,
    on_rename: Option<StringStringFn>,
    on_delete: Option<StringFn>,
    on_pin_toggle: Option<StringFn>,
    on_reorder: Option<ReorderListFn>,
}

#[derive(Clone)]
pub struct Sidebar {
    root: gtk::Box,
    list_box: gtk::ListBox,
    rows: Rc<RefCell<HashMap<String, gtk::ListBoxRow>>>,
    /// Needs-attention count label per workspace name. Hidden when zero.
    badges: Rc<RefCell<HashMap<String, gtk::Label>>>,
    /// Names in the visible (display) order.
    order: Rc<RefCell<Vec<String>>>,
    callbacks: Rc<RefCell<Callbacks>>,
    /// True while [`set_workspaces`] / [`set_active`] is mutating the
    /// list so the row-activated handler can suppress spurious callbacks.
    suppress_select: Rc<RefCell<bool>>,
}

impl Sidebar {
    pub fn new() -> Self {
        let list_box = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&list_box)
            .build();

        let new_btn = gtk::Button::builder()
            .label("New workspace")
            .icon_name("list-add-symbolic")
            .css_classes(["flat"])
            .build();

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        root.append(&scroller);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&new_btn);

        let me = Self {
            root,
            list_box,
            rows: Rc::new(RefCell::new(HashMap::new())),
            badges: Rc::new(RefCell::new(HashMap::new())),
            order: Rc::new(RefCell::new(Vec::new())),
            callbacks: Rc::new(RefCell::new(Callbacks::default())),
            suppress_select: Rc::new(RefCell::new(false)),
        };

        // Row activation → on_select callback.
        let cbs = me.callbacks.clone();
        let order = me.order.clone();
        let suppress = me.suppress_select.clone();
        me.list_box.connect_row_selected(move |_, row| {
            if *suppress.borrow() {
                return;
            }
            let Some(row) = row else { return };
            let idx = row.index();
            if idx < 0 {
                return;
            }
            let name = {
                let order = order.borrow();
                order.get(idx as usize).cloned()
            };
            if let Some(name) = name
                && let Some(cb) = cbs.borrow().on_select.as_ref()
            {
                cb(&name);
            }
        });

        // "New workspace" button → on_new callback.
        let cbs = me.callbacks.clone();
        new_btn.connect_clicked(move |_| {
            if let Some(cb) = cbs.borrow().on_new.as_ref() {
                cb();
            }
        });

        me
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn set_workspaces(&self, entries: &[WorkspaceRowData]) {
        *self.suppress_select.borrow_mut() = true;

        // Clear existing rows.
        let mut rows = self.rows.borrow_mut();
        for row in rows.values() {
            self.list_box.remove(row);
        }
        rows.clear();
        self.badges.borrow_mut().clear();
        self.order.borrow_mut().clear();

        for entry in entries {
            let label = gtk::Label::builder()
                .label(&entry.name)
                .xalign(0.0)
                .hexpand(true)
                .build();
            let badge = gtk::Label::builder()
                .label("")
                .css_classes(["sidebar-badge"])
                .visible(false)
                .build();
            let hbox = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(6)
                .margin_top(6)
                .margin_bottom(6)
                .margin_start(8)
                .margin_end(8)
                .build();
            if entry.pinned {
                let pin = gtk::Image::from_icon_name("starred-symbolic");
                pin.set_tooltip_text(Some("Pinned"));
                hbox.append(&pin);
            }
            hbox.append(&label);
            hbox.append(&badge);

            let row = gtk::ListBoxRow::builder().child(&hbox).build();
            install_row_context_menu(&row, &entry.name, &self.callbacks);
            self.list_box.append(&row);
            rows.insert(entry.name.clone(), row);
            self.badges.borrow_mut().insert(entry.name.clone(), badge);
            self.order.borrow_mut().push(entry.name.clone());
        }

        *self.suppress_select.borrow_mut() = false;
    }

    /// Set the unread-count badge for a workspace row. `0` hides it.
    pub fn set_workspace_badge(&self, name: &str, count: usize) {
        if let Some(label) = self.badges.borrow().get(name) {
            if count == 0 {
                label.set_visible(false);
                label.set_text("");
            } else {
                label.set_visible(true);
                label.set_text(&count.to_string());
            }
        }
    }

    pub fn set_active(&self, name: Option<&str>) {
        *self.suppress_select.borrow_mut() = true;
        match name.and_then(|n| self.rows.borrow().get(n).cloned()) {
            Some(row) => self.list_box.select_row(Some(&row)),
            None => self.list_box.unselect_all(),
        }
        *self.suppress_select.borrow_mut() = false;
    }

    pub fn on_select(&self, f: impl Fn(&str) + 'static) {
        self.callbacks.borrow_mut().on_select = Some(Box::new(f));
    }

    pub fn on_new(&self, f: impl Fn() + 'static) {
        self.callbacks.borrow_mut().on_new = Some(Box::new(f));
    }

    pub fn on_rename(&self, f: impl Fn(&str, &str) + 'static) {
        self.callbacks.borrow_mut().on_rename = Some(Box::new(f));
    }

    pub fn on_delete(&self, f: impl Fn(&str) + 'static) {
        self.callbacks.borrow_mut().on_delete = Some(Box::new(f));
    }

    pub fn on_pin_toggle(&self, f: impl Fn(&str) + 'static) {
        self.callbacks.borrow_mut().on_pin_toggle = Some(Box::new(f));
    }

    #[allow(dead_code)] // Drag-drop reorder UI wiring lands at M6 polish.
    pub fn on_reorder(&self, f: impl Fn(Vec<String>) + 'static) {
        self.callbacks.borrow_mut().on_reorder = Some(Box::new(f));
    }
}

fn install_row_context_menu(row: &gtk::ListBoxRow, name: &str, callbacks: &Rc<RefCell<Callbacks>>) {
    // Right-click → context menu with Rename / Pin (toggle) / Delete.
    let menu = build_row_menu();
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(row);
    popover.set_has_arrow(false);

    let gesture = gtk::GestureClick::new();
    gesture.set_button(3); // right-click
    let popover_clone = popover.clone();
    gesture.connect_pressed(move |_, _, x, y| {
        let rect = gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        popover_clone.set_pointing_to(Some(&rect));
        popover_clone.popup();
    });
    row.add_controller(gesture);

    // Long-press as a touchpad-friendly fallback.
    let long = gtk::GestureLongPress::new();
    let popover_clone = popover.clone();
    long.connect_pressed(move |_, x, y| {
        let rect = gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        popover_clone.set_pointing_to(Some(&rect));
        popover_clone.popup();
    });
    row.add_controller(long);

    // Install action handlers scoped to this row.
    let actions = gtk::gio::SimpleActionGroup::new();

    let name_owned = name.to_string();
    let cbs = callbacks.clone();
    let row_weak = row.downgrade();
    let act = gtk::gio::SimpleAction::new("rename", None);
    act.connect_activate(move |_, _| {
        // The rename action is async (needs a dialog), so we delegate
        // entirely to the on_rename callback which the shell wires up to
        // pop the dialog and call back into manager.rename().
        // We pass the same name as old + new placeholder; the shell will
        // ignore this and prompt the user. (We could add a separate
        // on_rename_request callback if we want stricter typing.)
        if let Some(cb) = cbs.borrow().on_rename.as_ref() {
            cb(&name_owned, &name_owned);
        }
        let _ = row_weak.upgrade();
    });
    actions.add_action(&act);

    let name_owned = name.to_string();
    let cbs = callbacks.clone();
    let act = gtk::gio::SimpleAction::new("pin-toggle", None);
    act.connect_activate(move |_, _| {
        if let Some(cb) = cbs.borrow().on_pin_toggle.as_ref() {
            cb(&name_owned);
        }
    });
    actions.add_action(&act);

    let name_owned = name.to_string();
    let cbs = callbacks.clone();
    let act = gtk::gio::SimpleAction::new("delete", None);
    act.connect_activate(move |_, _| {
        if let Some(cb) = cbs.borrow().on_delete.as_ref() {
            cb(&name_owned);
        }
    });
    actions.add_action(&act);

    row.insert_action_group("row", Some(&actions));
}

fn build_row_menu() -> gtk::gio::Menu {
    let menu = gtk::gio::Menu::new();
    menu.append(Some("Rename…"), Some("row.rename"));
    menu.append(Some("Pin / unpin"), Some("row.pin-toggle"));
    menu.append(Some("Delete…"), Some("row.delete"));
    menu
}
