//! The left and right sidebars: vault tree, search, backlinks and resizing.

#[allow(unused_imports)]
use super::*;

pub(super) const SIDEBAR_WIDTH_DEFAULT: Pixels = px(220.);

pub(super) const SIDEBAR_WIDTH_MIN: Pixels = px(140.);

pub(super) const SIDEBAR_WIDTH_MAX: Pixels = px(480.);

/// Grab area on the sidebar's inner edge; wider than the 1px border so it is easy to hit.
pub(super) const RESIZE_HANDLE_WIDTH: Pixels = px(6.);

/// Which sidebar a resize drag moves; also the drag payload.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SidebarSide {
    Left,
    Right,
}

/// Pointer x and sidebar width at mouse-down; the drag is measured from here, so
/// no layout positions (rails, window padding) need to be known.
#[derive(Clone, Copy)]
pub(super) struct ResizeStart {
    side: SidebarSide,
    pointer_x: Pixels,
    width: Pixels,
}

/// The (invisible) view GPUI shows under the cursor while dragging.
pub(super) struct ResizeGhost;

impl Render for ResizeGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// The left sidebar grows as the pointer moves right, the right one as it moves left.
pub(super) fn resized_width(start: ResizeStart, pointer_x: Pixels) -> Pixels {
    let delta = pointer_x - start.pointer_x;
    let width = match start.side {
        SidebarSide::Left => start.width + delta,
        SidebarSide::Right => start.width - delta,
    };
    width.clamp(SIDEBAR_WIDTH_MIN, SIDEBAR_WIDTH_MAX)
}

/// The lines that link to one note, shown under its backlinks; rebuilt (by
/// reading the source notes) only when its inputs change.
pub(super) struct BacklinkCache {
    target: String,
    generation: u64,
    sources: Vec<String>,
    contexts: Vec<jade_core::SourceContexts>,
}

/// Lines of context shown under each backlink.
pub(super) const BACKLINK_CONTEXT_LINES: usize = 3;

/// Whether a link written in note `from` points at something that exists.
/// Web links are not checked, and neither is a link within the same note.
pub(super) fn link_is_live(vault: &Vault, from: &str, kind: LinkKind, dest: &str) -> bool {
    !matches!(
        vault.link_destination(from, kind, dest),
        Destination::Missing
    )
}

impl JadeApp {
    /// Renders the notes and (non-collapsed) subfolders directly inside `parent`
    /// (vault root if `None`), indented by `depth`, recursing into subfolders.
    pub(super) fn render_tree_level(
        &self,
        parent: Option<&str>,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut folder_ids: Vec<&String> = self
            .vault
            .folders
            .iter()
            .filter(|id| parent_of(id) == parent)
            .collect();
        folder_ids.sort();

        let mut note_entries: Vec<(&String, &str)> = self
            .vault
            .notes
            .iter()
            .filter(|(id, _)| parent_of(id) == parent)
            .map(|(id, note)| (id, note.title.as_str()))
            .collect();
        note_entries.sort_by_key(|(_, title)| *title);

        let mut rows = Vec::new();
        for folder_id in folder_ids {
            rows.push(self.render_folder_row(folder_id, depth, cx));
            if !self.collapsed_folders.contains(folder_id.as_str()) {
                rows.extend(self.render_tree_level(Some(folder_id), depth + 1, cx));
            }
        }
        for (id, title) in note_entries {
            rows.push(self.render_note_row(id, title, depth, cx));
        }
        rows
    }

    pub(super) fn render_folder_row(
        &self,
        folder_id: &str,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self.collapsed_folders.contains(folder_id);
        let name = folder_id.rsplit('/').next().unwrap_or(folder_id);
        let disclosure = if collapsed { "\u{25b8}" } else { "\u{25be}" };
        let toggle_id = folder_id.to_string();
        let menu_parent = folder_id.to_string();
        let menu_name = name.to_string();
        let app_entity = cx.entity();

        div()
            .id(format!("folder-{folder_id}"))
            .test_support()
            .ml(px(depth as f32 * 14.))
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .child(format!("{disclosure} {name}"))
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.toggle_folder(&toggle_id, cx);
            }))
            .context_menu(move |menu, _window, _cx| {
                let note_entity = app_entity.clone();
                let note_parent = menu_parent.clone();
                let folder_entity = app_entity.clone();
                let folder_parent = menu_parent.clone();
                let rename_entity = app_entity.clone();
                let rename_id = menu_parent.clone();
                let rename_name = menu_name.clone();
                let delete_entity = app_entity.clone();
                let delete_id = menu_parent.clone();
                let delete_name = menu_name.clone();
                menu.item(
                    PopupMenuItem::new("New note").on_click(move |_, window, cx| {
                        note_entity.update(cx, |app, cx| {
                            app.create_note(Some(&note_parent), window, cx);
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new("New folder").on_click(move |_, window, cx| {
                        JadeApp::open_create_folder_dialog(
                            Some(folder_parent.clone()),
                            folder_entity.clone(),
                            window,
                            cx,
                        );
                    }),
                )
                .item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
                    JadeApp::open_rename_folder_dialog(
                        rename_id.clone(),
                        rename_name.clone(),
                        rename_entity.clone(),
                        window,
                        cx,
                    );
                }))
                .item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
                    JadeApp::open_delete_folder_dialog(
                        delete_id.clone(),
                        delete_name.clone(),
                        delete_entity.clone(),
                        window,
                        cx,
                    );
                }))
            })
            .debug_name_with(|| format!("folder-{folder_id}").into())
            .into_any_element()
    }

    pub(super) fn render_note_row(
        &self,
        id: &str,
        title: &str,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_selected = self.selected_id() == Some(id);
        let item_id = id.to_string();
        let menu_id = id.to_string();
        let menu_title = title.to_string();
        let app_entity = cx.entity();

        let mut item = div()
            .id(id.to_string())
            .test_support()
            .ml(px(depth as f32 * 14.))
            .px_2()
            .py_1()
            .rounded_md()
            .child(title.to_string());
        if is_selected {
            item = item.bg(ThemeEngine::color(cx, Role::SurfaceHover));
        }
        item = item.on_click(cx.listener(move |this, _event, window, cx| {
            this.open_note(item_id.clone(), window, cx);
            this.sidebar_focus.focus(window, cx);
        }));

        item.context_menu(move |menu, _window, _cx| {
            let open_entity = app_entity.clone();
            let open_id = menu_id.clone();
            let new_tab_entity = app_entity.clone();
            let new_tab_id = menu_id.clone();
            let rename_entity = app_entity.clone();
            let rename_id = menu_id.clone();
            let rename_title = menu_title.clone();
            let delete_entity = app_entity.clone();
            let delete_id = menu_id.clone();
            let delete_title = menu_title.clone();
            menu.item(PopupMenuItem::new("Open").on_click(move |_, window, cx| {
                open_entity.update(cx, |app, cx| app.open_note(open_id.clone(), window, cx));
            }))
            .item(
                PopupMenuItem::new("Open in new tab").on_click(move |_, window, cx| {
                    new_tab_entity.update(cx, |app, cx| {
                        app.open_note_in_new_tab(new_tab_id.clone(), window, cx);
                    });
                }),
            )
            .item(PopupMenuItem::new("Rename").on_click(move |_, window, cx| {
                JadeApp::open_rename_dialog(
                    rename_id.clone(),
                    rename_title.clone(),
                    rename_entity.clone(),
                    window,
                    cx,
                );
            }))
            .item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
                JadeApp::open_delete_dialog(
                    delete_id.clone(),
                    delete_title.clone(),
                    delete_entity.clone(),
                    window,
                    cx,
                );
            }))
        })
        .debug_name_with(|| format!("note-{id}").into())
        .into_any_element()
    }

    /// The right sidebar's body: notes linking to `note_id`.
    pub(super) fn render_backlinks_panel(&self, note_id: &str, cx: &mut Context<Self>) -> Div {
        let mut backlink_entries: Vec<(String, String)> = self
            .vault
            .backlinks()
            .get(note_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|source_id| {
                self.vault
                    .note(&source_id)
                    .map(|n| (source_id, n.title.clone()))
            })
            .collect();
        backlink_entries.sort_by(|a, b| a.1.cmp(&b.1));

        let mut backlinks_panel = div().flex().flex_col().gap_1().child(
            div()
                .font_weight(FontWeight::BOLD)
                .child(format!("Backlinks ({})", backlink_entries.len())),
        );

        if backlink_entries.is_empty() {
            backlinks_panel = backlinks_panel.child(
                div()
                    .text_color(ThemeEngine::color(cx, Role::TextMuted))
                    .child("No backlinks yet."),
            );
        } else {
            let row_hover = ThemeEngine::color(cx, Role::Surface);
            let muted = ThemeEngine::color(cx, Role::TextMuted);
            let sources: Vec<String> = backlink_entries.iter().map(|(id, _)| id.clone()).collect();
            let mut cache = self.backlink_cache.borrow_mut();
            let stale = cache.as_ref().is_none_or(|c| {
                c.target != note_id
                    || c.generation != self.content_generation
                    || c.sources != sources
            });
            if stale {
                *cache = Some(BacklinkCache {
                    target: note_id.to_string(),
                    generation: self.content_generation,
                    contexts: self
                        .vault
                        .link_contexts(note_id, &sources, BACKLINK_CONTEXT_LINES),
                    sources,
                });
            }
            let contexts = cache.as_ref().map(|c| c.contexts.as_slice()).unwrap_or(&[]);
            for (source_id, title) in backlink_entries {
                let target_id = source_id.clone();
                let mut entry = div().flex().flex_col().child(
                    div()
                        .id(format!("backlink-{source_id}"))
                        .test_support()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(move |style| style.bg(row_hover))
                        .child(title)
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            this.open_note(target_id.clone(), window, cx);
                        }))
                        .debug_name_with(|| format!("backlink-{source_id}").into()),
                );
                let Some(found) = contexts.iter().find(|c| c.source == source_id) else {
                    backlinks_panel = backlinks_panel.child(entry);
                    continue;
                };
                for context in &found.contexts {
                    let (open_id, line) = (source_id.clone(), context.line);
                    entry = entry.child(
                        div()
                            .id(format!("backlink-{source_id}-line-{line}"))
                            .test_support()
                            .pl_4()
                            .pr_2()
                            .text_xs()
                            .text_color(muted)
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |style| style.bg(row_hover))
                            .child(context.snippet.clone())
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                this.open_note(open_id.clone(), window, cx);
                                this.jump_to_line(line, cx);
                            }))
                            .debug_name_with(|| format!("backlink-{source_id}-line-{line}").into()),
                    );
                }
                if found.total > found.contexts.len() {
                    entry = entry.child(
                        div()
                            .pl_4()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("+{} more", found.total - found.contexts.len())),
                    );
                }
                backlinks_panel = backlinks_panel.child(entry);
            }
        }

        backlinks_panel
    }

    /// The draggable strip on a sidebar's inner edge.
    pub(super) fn render_resize_handle(
        &self,
        side: SidebarSide,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (id, width) = match side {
            SidebarSide::Left => ("sidebar-resize", self.sidebar_width),
            SidebarSide::Right => ("backlinks-resize", self.backlinks_width),
        };
        let hover = ThemeEngine::faded(cx, Role::Accent, Alpha::AccentHover);
        let pressed = ThemeEngine::faded(cx, Role::Accent, Alpha::AccentActive);
        let handle = div()
            .id(id)
            .test_support()
            .absolute()
            .top_0()
            .bottom_0()
            .w(RESIZE_HANDLE_WIDTH)
            .cursor_col_resize()
            .hover(move |style| style.bg(hover))
            .active(move |style| style.bg(pressed));
        let handle = match side {
            SidebarSide::Left => handle.right_0(),
            SidebarSide::Right => handle.left_0(),
        };
        handle
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, _cx| {
                    this.resize_start = Some(ResizeStart {
                        side,
                        pointer_x: event.position.x,
                        width,
                    });
                }),
            )
            .on_drag(side, |_, _, _, cx| cx.new(|_| ResizeGhost))
            .debug_name(id)
            .into_any_element()
    }

    pub(super) fn resize_sidebar(
        &mut self,
        pointer_x: Pixels,
        side: SidebarSide,
        cx: &mut Context<Self>,
    ) {
        let Some(start) = self.resize_start.filter(|start| start.side == side) else {
            return;
        };
        let width = resized_width(start, pointer_x);
        assert!((SIDEBAR_WIDTH_MIN..=SIDEBAR_WIDTH_MAX).contains(&width));
        let target = match side {
            SidebarSide::Left => &mut self.sidebar_width,
            SidebarSide::Right => &mut self.backlinks_width,
        };
        if *target != width {
            *target = width;
            cx.notify();
        }
    }

    pub(super) fn render_sidebar_tab_button(
        &self,
        tab: SidebarTab,
        label: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_active = self.sidebar_tab == tab;
        let id = match tab {
            SidebarTab::Vault => "sidebar-tab-vault",
            SidebarTab::Search => "sidebar-tab-search",
        };

        let mut item = div()
            .id(id)
            .test_support()
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .child(label.to_string());
        if is_active {
            item = item
                .bg(ThemeEngine::color(cx, Role::SurfaceHover))
                .font_weight(FontWeight::BOLD);
        }
        item.on_click(cx.listener(move |this, _event, _window, cx| {
            this.sidebar_tab = tab;
            cx.notify();
        }))
        .debug_name(id)
        .into_any_element()
    }
}
