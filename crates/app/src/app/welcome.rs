//! The first-start dialog that asks where the notes live.

#[allow(unused_imports)]
use super::*;

impl JadeApp {
    /// Cannot be dismissed: without a vault there is nothing to show, and the
    /// window's own close button still quits.
    pub(super) fn open_welcome_dialog(
        app: Entity<Self>,
        config_dirs: Option<ConfigDirs>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let suggestion = crate::vault_path::suggestion(::dirs::document_dir(), ::dirs::home_dir())
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(suggestion));
        let problem: Entity<Option<SharedString>> = cx.new(|_| None);
        window.open_dialog(cx, move |dialog, _window, cx| {
            let muted = ThemeEngine::color(cx, Role::TextMuted);
            let error = ThemeEngine::color(cx, Role::Error);
            let message = problem.read(cx).clone();
            let commit = {
                let (input, problem, app) = (input.clone(), problem.clone(), app.clone());
                let config_dirs = config_dirs.clone();
                move |window: &mut Window, cx: &mut App| {
                    Self::commit_vault(&input, &problem, &app, config_dirs.as_ref(), window, cx)
                }
            };
            dialog
                .title("Welcome to Jade")
                .close_button(false)
                .overlay_closable(false)
                .keyboard(false)
                .child(
                    div()
                        .id("welcome-panel")
                        .test_support()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_color(muted)
                                .child("Where do you want to store your notes?"),
                        )
                        .child(
                            Input::new(&input)
                                .id("vault-path-input")
                                .debug_name("vault-path-input"),
                        )
                        .when_some(message, |this, message| {
                            this.child(div().text_color(error).child(message))
                        })
                        .debug_name("welcome-panel"),
                )
                // Enter confirms through `on_ok`; see the rename dialog.
                .on_ok({
                    let commit = commit.clone();
                    move |_, window, cx| commit(window, cx)
                })
                .footer(
                    DialogFooter::new().child(
                        Button::new("confirm-vault")
                            .label("OK")
                            .on_click(move |_, window, cx| {
                                if commit(window, cx) {
                                    window.close_dialog(cx);
                                }
                            })
                            .debug_name("confirm-vault"),
                    ),
                )
        });
    }

    /// Checks the typed path, creates the folder, saves it to `config.toml`
    /// and opens it. On failure the dialog stays open showing why.
    fn commit_vault(
        input: &Entity<InputState>,
        problem: &Entity<Option<SharedString>>,
        app: &Entity<Self>,
        config_dirs: Option<&ConfigDirs>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let text = input.read(cx).value().to_string();
        let saved =
            crate::vault_path::parse_input(&text, ::dirs::home_dir().as_deref()).and_then(|path| {
                crate::vault_path::prepare(&path)?;
                if let Some(dirs) = config_dirs {
                    crate::theme::remember_vault(dirs, &path)?;
                }
                Ok(path)
            });
        match saved {
            Ok(path) => {
                app.update(cx, |app, cx| app.open_vault(path, cx));
                true
            }
            Err(message) => {
                problem.update(cx, |problem, cx| {
                    *problem = Some(message.into());
                    cx.notify();
                });
                window.refresh();
                false
            }
        }
    }
}
