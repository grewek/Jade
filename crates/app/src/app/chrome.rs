//! The tab bar and the status bar.

#[allow(unused_imports)]
use super::*;

/// `1234567` as `1,234,567`.
pub(super) fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    grouped
}

impl JadeApp {
    /// The row of open notes above the editor.
    pub(super) fn render_tab_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let bar = ThemeEngine::color(cx, Role::BackgroundSecondary);
        let border = ThemeEngine::color(cx, Role::Border);
        let tabs: Vec<AnyElement> = (0..self.tabs.len())
            .map(|ix| self.render_tab(ix, cx))
            .collect();
        let scroll = self.tabs.scroll().clone();
        let strip = div()
            .id("tab-bar")
            .test_support()
            .flex()
            .flex_row()
            .overflow_x_scroll()
            .track_scroll(self.tabs.scroll())
            .bg(bar)
            // A plain mouse wheel only scrolls vertically; a row of tabs that
            // cannot be reached with it would be stuck.
            .on_scroll_wheel(
                cx.listener(move |_this, event: &ScrollWheelEvent, window, cx| {
                    let delta = event.delta.pixel_delta(window.line_height());
                    if delta.x != px(0.) || delta.y == px(0.) {
                        return;
                    }
                    let mut offset = scroll.offset();
                    offset.x = (offset.x + delta.y).clamp(-scroll.max_offset().x, px(0.));
                    scroll.set_offset(offset);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .children(tabs)
            .debug_name("tab-bar");
        // The bar's width is the editor column's; the scrollbar overlays it
        // rather than taking height.
        div()
            .relative()
            .flex_none()
            .border_b_1()
            .border_color(border)
            .child(strip)
            .child(
                div().absolute().inset_0().child(
                    Scrollbar::horizontal(self.tabs.scroll())
                        .id("tab-scrollbar")
                        .viewport_from_layout(),
                ),
            )
            .into_any_element()
    }

    /// Word and character counts of the shown note, under the editor.
    pub(super) fn render_status_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editor = self.editor()?.read(cx);
        let (words, chars) = (editor.word_count(), editor.char_count());
        let border = ThemeEngine::color(cx, Role::Border);
        let muted = ThemeEngine::color(cx, Role::TextMuted);
        let plural = |n: usize, one: &str, many: &str| {
            format!("{} {}", group_digits(n), if n == 1 { one } else { many })
        };
        Some(
            div()
                .id("status-bar")
                .test_support()
                .flex()
                .flex_row()
                .flex_none()
                .justify_end()
                .gap_4()
                .px_3()
                .py_1()
                .border_t_1()
                .border_color(border)
                .text_xs()
                .text_color(muted)
                .child(plural(words, "word", "words"))
                .child(plural(chars, "character", "characters"))
                // Placeholder for the grammar/spell checker, which is its own
                // feature (Backlog): the icon alone, with no behaviour yet.
                .child(
                    div()
                        .id("grammar-check")
                        .test_support()
                        .child(IconName::CircleCheck)
                        .debug_name("grammar-check"),
                )
                .debug_name("status-bar")
                .into_any_element(),
        )
    }

    /// One tab: click to show it, the cross or a middle click to close it.
    pub(super) fn render_tab(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[ix];
        let border = ThemeEngine::color(cx, Role::Border);
        let (background, foreground) = if self.tabs.active_index() == Some(ix) {
            (
                ThemeEngine::color(cx, Role::Background),
                ThemeEngine::color(cx, Role::Text),
            )
        } else {
            (
                gpui_kit::transparent_black(),
                ThemeEngine::color(cx, Role::TextMuted),
            )
        };
        let title = self
            .vault
            .note(&tab.note_id)
            .map_or_else(|| tab.note_id.clone(), |note| note.title.clone());
        let id = tab.note_id.as_str();
        let close = Button::new(format!("tab-close-{id}"))
            .icon(IconName::Close)
            .ghost()
            .xsmall()
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.close_tab(ix, window, cx);
            }))
            .debug_name_with(|| format!("tab-close-{id}").into());
        div()
            .id(format!("tab-{id}"))
            .test_support()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap_1()
            .pl_3()
            .pr_1()
            .py_1()
            .max_w(px(200.))
            .bg(background)
            .text_color(foreground)
            .border_r_1()
            .border_color(border)
            .cursor_pointer()
            .child(div().min_w_0().truncate().child(title))
            // The wrapper keeps the cross's press from also starting a click on
            // the tab (which would re-activate it after it closed).
            .child(
                div()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(close),
            )
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.activate_tab(ix, window, cx);
            }))
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |this, _event, window, cx| {
                    this.close_tab(ix, window, cx);
                }),
            )
            .debug_name_with(|| format!("tab-{id}").into())
            .into_any_element()
    }
}
