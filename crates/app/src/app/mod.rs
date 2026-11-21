use std::path::PathBuf;

use crate::debug_server::{self, targets::DebugName};
use crate::text_editor::{Editor, EditorEvent};
use crate::theme::{Alpha, ConfigDirs, Role, ThemeEngine};
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{IconName, IndexPath, Sizable as _, Theme, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jade_core::link_scan::LinkKind;
use jade_core::{Destination, Vault, search_targets};

use crate::text_editor::TextSettings;

mod chrome;
mod dialogs;
mod notes;
mod sidebar;
mod tabs;

pub(crate) use sidebar::SidebarSide;
#[allow(unused_imports)]
use {chrome::*, dialogs::*, sidebar::*};

/// The window's `app_id`; must match the desktop entry's file name.
const APP_ID: &str = "jade";

/// IBM Plex — the typeface Zed itself uses — bundled under the SIL Open Font
/// License (OFL 1.1, see `assets/fonts/*/LICENSE.txt`), which explicitly
/// permits embedding/redistributing it with an application.
const IBM_PLEX_SANS: &str = "IBM Plex Sans";
const IBM_PLEX_MONO: &str = "IBM Plex Mono";

fn install_fonts(cx: &mut App) {
    use std::borrow::Cow;

    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(
                include_bytes!("../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf")
                    .as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../../assets/fonts/ibm-plex-sans/IBMPlexSans-Italic.ttf")
                    .as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf")
                    .as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../../assets/fonts/ibm-plex-mono/IBMPlexMono-Italic.ttf")
                    .as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../../assets/fonts/ibm-plex-mono/IBMPlexMono-Bold.ttf").as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../../assets/fonts/ibm-plex-mono/IBMPlexMono-BoldItalic.ttf")
                    .as_slice(),
            ),
        ])
        .expect("failed to load bundled IBM Plex fonts");

    let theme = Theme::global_mut(cx);
    theme.font_family = IBM_PLEX_SANS.into();
    theme.mono_font_family = IBM_PLEX_MONO.into();
}

/// Idle time after the last edit before the note is written to disk.
const AUTOSAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

actions!(jade, [RenameNote, DeleteNote, OpenCommandPalette]);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SidebarTab {
    #[default]
    Vault,
    Search,
}

/// One open note. The editor is Jade's own `text_editor::Editor` (not
/// `gpui-component`'s `Textarea`); each tab keeps its own, so cursor, scroll and
/// undo survive switching tabs.
pub(crate) struct Tab {
    pub(crate) note_id: String,
    editor: Entity<Editor>,
    /// Set by the editor's `Changed` event, cleared when the autosave writes it.
    dirty: bool,
    _subscription: Subscription,
}

pub(crate) struct JadeApp {
    vault: Vault,
    /// Open notes, one tab each; at most one tab per note.
    pub(crate) tabs: Vec<Tab>,
    /// Index into `tabs` of the tab being shown.
    pub(crate) active_tab: Option<usize>,
    /// Horizontal scroll of the tab bar, so the active tab can be brought into view.
    tab_scroll: ScrollHandle,
    /// Pending debounced save of the dirty tabs; dropping it cancels the wait.
    autosave_task: Option<Task<()>>,
    _quit_subscription: Subscription,
    pub(crate) error: Option<String>,
    /// Bumped whenever a note's text on disk or the set of notes changes;
    /// tells `backlink_cache` it is out of date.
    content_generation: u64,
    backlink_cache: std::cell::RefCell<Option<BacklinkCache>>,
    sidebar_focus: FocusHandle,
    collapsed_folders: std::collections::HashSet<String>,
    pub(crate) sidebar_tab: SidebarTab,
    /// Hides the whole sidebar (tabs, vault tree, search); the toggle rail stays.
    pub(crate) sidebar_collapsed: bool,
    /// Hides the right sidebar (backlinks); its toggle rail stays.
    pub(crate) backlinks_collapsed: bool,
    pub(crate) sidebar_width: Pixels,
    pub(crate) backlinks_width: Pixels,
    resize_start: Option<ResizeStart>,
    search_input: Entity<InputState>,
    pub(crate) search_query: String,
    /// Search reads note files, so results are computed once per change
    /// (query or note text/set), never per frame: mutations set
    /// `search_stale` and `render` recomputes when the tab is showing.
    pub(crate) search_results: Vec<(String, String)>,
    search_stale: bool,
    /// Bumped per started search; a finished one whose generation is no
    /// longer current is dropped.
    search_generation: u64,
    /// The in-flight background search; dropping it cancels the wait.
    search_task: Option<Task<()>>,
    _search_subscription: Subscription,
}

/// The direct parent folder id of a note/folder id, or `None` at the vault root.
fn parent_of(id: &str) -> Option<&str> {
    id.rsplit_once('/').map(|(parent, _)| parent)
}

impl JadeApp {
    pub(crate) fn new(vault_path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sidebar_focus = cx.focus_handle();

        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search notes..."));
        let search_subscription = cx.subscribe(&search_input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = input.read(cx).value().to_string();
                this.search_stale = true;
                cx.notify();
            }
        });

        let (vault, error) = match Vault::open(&vault_path) {
            Ok(vault) => (vault, None),
            Err(err) => (Vault::empty(vault_path), Some(err.to_string())),
        };

        Self {
            vault,
            tabs: Vec::new(),
            active_tab: None,
            tab_scroll: ScrollHandle::new(),
            autosave_task: None,
            // Debounced saves can still be pending at quit.
            _quit_subscription: cx.on_app_quit(|this, cx| {
                this.persist_all_tabs(cx);
                async {}
            }),
            error,
            sidebar_focus,
            collapsed_folders: Default::default(),
            sidebar_tab: SidebarTab::default(),
            sidebar_collapsed: false,
            content_generation: 0,
            backlink_cache: std::cell::RefCell::new(None),
            backlinks_collapsed: false,
            sidebar_width: SIDEBAR_WIDTH_DEFAULT,
            backlinks_width: SIDEBAR_WIDTH_DEFAULT,
            resize_start: None,
            search_input,
            search_query: String::new(),
            search_results: Vec::new(),
            search_stale: true,
            search_generation: 0,
            search_task: None,
            _search_subscription: search_subscription,
        }
    }
}

impl Render for JadeApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        debug_server::targets::begin_frame();
        let tab_bar = div()
            .flex()
            .flex_row()
            .gap_1()
            .mb_2()
            .child(self.render_sidebar_tab_button(SidebarTab::Vault, "Vault", cx))
            .child(self.render_sidebar_tab_button(SidebarTab::Search, "Search", cx));

        // The "NoteList" key context (and its F2/Backspace bindings) is scoped to
        // just the vault tree, not the whole sidebar — otherwise Backspace while
        // typing in the search input above would delete the selected note instead
        // of erasing a character.
        let sidebar_content: AnyElement = match self.sidebar_tab {
            SidebarTab::Vault => {
                let tree_rows = self.render_tree_level(None, 0, cx);
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .key_context("NoteList")
                    .track_focus(&self.sidebar_focus)
                    .on_action(cx.listener(Self::rename_selected))
                    .on_action(cx.listener(Self::delete_selected))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_1()
                            .child(
                                Button::new("new-note")
                                    .label("New note")
                                    .on_click(cx.listener(|this, _event, window, cx| {
                                        this.create_note(None, window, cx);
                                    }))
                                    .debug_name("new-note"),
                            )
                            .child(
                                Button::new("new-folder")
                                    .label("New folder")
                                    .on_click(cx.listener(|_this, _event, window, cx| {
                                        let app_entity = cx.entity();
                                        JadeApp::open_create_folder_dialog(
                                            None, app_entity, window, cx,
                                        );
                                    }))
                                    .debug_name("new-folder"),
                            ),
                    )
                    .children(tree_rows)
                    .into_any_element()
            }
            SidebarTab::Search => self.render_search_panel(cx),
        };

        // Without an explicit color the 1px borders below are not visible.
        let divider = ThemeEngine::color(cx, Role::Border);
        let secondary = ThemeEngine::color(cx, Role::BackgroundSecondary);
        let tertiary = ThemeEngine::color(cx, Role::BackgroundTertiary);
        let sidebar = div()
            .relative()
            .flex()
            .flex_col()
            .flex_none()
            .gap_1()
            .w(self.sidebar_width)
            .p_2()
            .bg(secondary)
            .border_r_1()
            .border_color(divider)
            .child(tab_bar)
            .child(sidebar_content)
            .child(self.render_resize_handle(SidebarSide::Left, cx));

        let content: AnyElement = if let Some(err) = &self.error {
            div()
                .p_4()
                .child(format!("Could not open vault: {err}"))
                .into_any_element()
        } else if self.active_tab.is_some() {
            let mut editor_pane = div()
                .id("note-editor-pane")
                .size_full()
                .overflow_y_scroll()
                .children(self.editor().cloned());
            // Tracked so `text_editor::element`'s `autoscroll_to_cursor` can
            // scroll this container itself (see `Editor::scroll_handle`'s
            // doc comment) — without this, moving the cursor below the
            // visible area via the keyboard has nothing to adjust.
            // The scrollbar overlays a wrapper rather than the scroll container,
            // or it would scroll away with the content. `Scrollbar`'s default
            // mode shows it while scrolling and fades it out after idle.
            // Padded on the right: the scrollbar (absolute, over the padding box)
            // sits there, beside the pane and not over its text. The pane itself
            // stays unpadded, since the editor takes its width from the pane's.
            let mut editor_area = div().relative().flex_1().min_h_0().pr(px(16.));
            if let Some(editor) = self.editor() {
                let handle = editor.read(cx).scroll_handle();
                editor_pane = editor_pane.track_scroll(handle);
                editor_area = editor_area
                    .child(editor_pane.debug_name("editor-pane"))
                    .child(
                        div().absolute().inset_0().child(
                            Scrollbar::vertical(handle)
                                .id("editor-scrollbar")
                                .viewport_from_layout(),
                        ),
                    );
            } else {
                editor_area = editor_area.child(editor_pane.debug_name("editor-pane"));
            }

            div()
                .flex()
                .flex_col()
                .size_full()
                .p_4()
                .child(editor_area)
                .into_any_element()
        } else {
            div()
                .p_4()
                .child("Select a note, or create a new one.")
                .into_any_element()
        };

        let toggle_icon = if self.sidebar_collapsed {
            IconName::PanelLeftOpen
        } else {
            IconName::PanelLeftClose
        };
        let toggle = Button::new("toggle-sidebar")
            .icon(toggle_icon)
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.sidebar_collapsed = !this.sidebar_collapsed;
                cx.notify();
            }))
            .debug_name("toggle-sidebar");
        let rail = div()
            .flex()
            .flex_col()
            .flex_none()
            .p_1()
            .bg(tertiary)
            .border_r_1()
            .border_color(divider)
            .child(toggle);

        let backlinks_toggle = Button::new("toggle-backlinks")
            .icon(if self.backlinks_collapsed {
                IconName::PanelRightOpen
            } else {
                IconName::PanelRightClose
            })
            .on_click(cx.listener(|this, _event, _window, cx| {
                this.backlinks_collapsed = !this.backlinks_collapsed;
                cx.notify();
            }))
            .debug_name("toggle-backlinks");
        let right_rail = div()
            .flex()
            .flex_col()
            .flex_none()
            .p_1()
            .bg(tertiary)
            .border_l_1()
            .border_color(divider)
            .child(backlinks_toggle);
        let right_sidebar = div()
            .relative()
            .flex()
            .flex_col()
            .gap_1()
            .w(self.backlinks_width)
            .flex_none()
            .p_2()
            .bg(secondary)
            .border_l_1()
            .border_color(divider)
            .children(
                self.selected_id()
                    .filter(|_| self.error.is_none())
                    .map(|id| self.render_backlinks_panel(id, cx)),
            )
            .child(self.render_resize_handle(SidebarSide::Right, cx));

        div()
            .flex()
            .flex_row()
            .size_full()
            .bg(ThemeEngine::color(cx, Role::Background))
            .text_color(ThemeEngine::color(cx, Role::Text))
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<SidebarSide>, _window, cx| {
                    let side = *event.drag(cx);
                    this.resize_sidebar(event.event.position.x, side, cx);
                }),
            )
            .child(rail)
            .when(!self.sidebar_collapsed, |this| {
                this.child(sidebar.debug_name("sidebar"))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .when(!self.tabs.is_empty(), |this| {
                        this.child(self.render_tab_bar(cx))
                    })
                    .child(div().flex_1().min_h_0().child(content))
                    .children(self.render_status_bar(cx)),
            )
            .when(!self.backlinks_collapsed, |this| {
                this.child(right_sidebar.debug_name("backlinks-sidebar"))
            })
            .child(right_rail)
    }
}

/// Opens the window and runs the app until it is closed; the `jade-app` binary
/// is just this call.
pub fn run() {
    let vault_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("vault"));

    debug_server::start_if_enabled();

    // Registers the bundled Lucide icon SVGs (`Dialog`'s own close "x",
    // `Button::icon(...)`, etc.) with GPUI's asset loader — without this,
    // every `IconName` renders as empty (the button itself still paints,
    // just with nothing inside it), not just the close button specifically.
    gpui_kit::application()
        .with_assets(crate::assets::JadeAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            install_fonts(cx);
            let config_dirs = ConfigDirs::from_environment();
            let mut notices = ThemeEngine::install(config_dirs.clone(), cx);
            crate::text_editor::TextSettings::install(config_dirs.as_ref(), cx);
            if let Some(dirs) = config_dirs {
                notices.extend(crate::theme::watch(dirs, cx));
            }
            for notice in notices {
                eprintln!("jade: {notice}");
            }
            cx.bind_keys([
                KeyBinding::new("f2", RenameNote, Some("NoteList")),
                KeyBinding::new("backspace", DeleteNote, Some("NoteList")),
                KeyBinding::new("ctrl-p", OpenCommandPalette, None),
            ]);
            cx.bind_keys(crate::text_editor::key_bindings());
            bind_open_command_palette(cx);
            // Wayland picks the window icon from the desktop entry whose name
            // is this id (`packaging/jade.desktop`); the compositor ignores
            // any icon the app sends.
            let options = WindowOptions {
                app_id: Some(APP_ID.to_owned()),
                ..WindowOptions::default()
            };
            let (_, app) = gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| JadeApp::new(vault_path.clone(), window, cx))
            })
            .expect("failed to open window");
            debug_server::install(&app, cx);
        });
}

/// Registers `OpenCommandPalette`'s handler globally (`cx.on_action`, not an
/// element-attached `.on_action(cx.listener(...))`) — see `JadeApp::
/// open_command_palette`'s own doc comment for why this specific mechanism
/// is required for Ctrl+P to work with nothing in the app currently
/// focused. Split out so both `main()` and the test helper `open_app` call
/// the exact same registration, same reasoning `cx.bind_keys([...])` is
/// already duplicated between the two rather than risking them drifting
/// apart.
fn bind_open_command_palette(cx: &mut App) {
    cx.on_action(|_: &OpenCommandPalette, cx| {
        // Not `cx.active_window()` — that's a genuine OS-level "which
        // window has platform focus" query (`App::active_window` delegates
        // straight to `self.platform.active_window()`), which is legitimately
        // always `None` under the headless test platform (no real window
        // manager to ask), and not something to lean on for a single-window
        // app regardless. `cx.windows()` just lists every window GPUI
        // itself has open, with no OS-focus dependency at all.
        let Some(window_handle) = cx.windows().into_iter().next() else {
            return;
        };
        cx.defer(move |cx| {
            _ = window_handle.update(cx, |_, window, cx| {
                JadeApp::open_command_palette(window, cx);
            });
        });
    });
}

// Deliberately not `use super::*;` / `use gpui_kit::*;` here: with `test-support`
// enabled, that glob also re-exports GPUI's `test` attribute macro, which then
// shadows the plain `#[test]` its own expansion relies on and recurses forever.

#[cfg(test)]
mod tests;
