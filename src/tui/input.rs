use crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};

use super::app::{App, Screen, StatusKind};
use crate::provider::CUSTOM_URL_ID;

/// Map a crossterm key event to an `Action` for the current screen.
/// Pure function: same input + same app state = same action. This is
/// the only place that touches `KeyCode`; everything else takes
/// `Action`s and renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    NextProvider,
    PrevProvider,
    NextModel,
    PrevModel,
    Select,
    BackToProvider,
    BackToModel,
    GoStatus,
    AddCustom,
    RemoveCustom,
    ToggleOnOff,
    Verify,
    Refresh,
    Char(char),
    Backspace,
    SubmitInput,
    CancelInput,
}

pub fn map_key(app: &App, key: KeyEvent) -> Option<Action> {
    // Ctrl-C always quits.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }

    // Input mode owns its own keys.
    if app.input_active {
        return match key.code {
            KeyCode::Enter => Some(Action::SubmitInput),
            KeyCode::Esc => Some(Action::CancelInput),
            KeyCode::Backspace => Some(Action::Backspace),
            KeyCode::Char(c) => Some(Action::Char(c)),
            _ => None,
        };
    }

    // Global keys.
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => return Some(Action::Quit),
        KeyCode::Tab => {
            return Some(match app.screen {
                Screen::Provider => Action::GoStatus,
                Screen::Model => Action::BackToProvider,
                Screen::Status => Action::BackToModel,
            });
        }
        KeyCode::Esc => {
            return Some(match app.screen {
                Screen::Provider => Action::Quit,
                Screen::Model => Action::BackToProvider,
                Screen::Status => Action::BackToModel,
            });
        }
        KeyCode::Enter => return Some(Action::Select),
        KeyCode::Char('j') | KeyCode::Down => {
            return Some(match app.screen {
                Screen::Provider => Action::NextProvider,
                Screen::Model => Action::NextModel,
                Screen::Status => Action::Refresh,
            });
        }
        KeyCode::Char('k') | KeyCode::Up => {
            return Some(match app.screen {
                Screen::Provider => Action::PrevProvider,
                Screen::Model => Action::PrevModel,
                Screen::Status => Action::Refresh,
            });
        }
        _ => {}
    }

    // Per-screen keys.
    match app.screen {
        Screen::Provider => match key.code {
            KeyCode::Char('a') => Some(Action::AddCustom),
            KeyCode::Char('d') => Some(Action::RemoveCustom),
            _ => None,
        },
        Screen::Model => None,
        Screen::Status => match key.code {
            KeyCode::Char('o') => Some(Action::ToggleOnOff),
            KeyCode::Char('v') => Some(Action::Verify),
            KeyCode::Char('r') => Some(Action::Refresh),
            _ => None,
        },
    }
}

/// Apply an action to the app. Returns true if the event loop should
/// continue, false if the app should quit.
pub fn apply(app: &mut App, action: Action) -> bool {
    if action == Action::Quit {
        app.should_quit = true;
        return false;
    }
    if app.input_active {
        match action {
            Action::SubmitInput => {
                if app.input_prompt == "Model id: " {
                    app.input_active = false;
                    app.apply_selection();
                } else {
                    app.commit_custom_url();
                }
            }
            Action::CancelInput => {
                app.input_active = false;
                app.input_buffer.clear();
            }
            Action::Backspace => {
                app.input_buffer.pop();
            }
            Action::Char(c) => {
                app.input_buffer.push(c);
            }
            _ => {}
        }
        return true;
    }

    match action {
        Action::NextProvider => {
            let next = (app.provider_index + 1) % app.providers.len();
            app.select_provider(next);
        }
        Action::PrevProvider => {
            let prev = if app.provider_index == 0 {
                app.providers.len() - 1
            } else {
                app.provider_index - 1
            };
            app.select_provider(prev);
        }
        Action::NextModel => {
            if !app.models.is_empty() {
                app.model_index = (app.model_index + 1) % app.models.len();
            }
        }
        Action::PrevModel => {
            if !app.models.is_empty() {
                app.model_index = if app.model_index == 0 {
                    app.models.len() - 1
                } else {
                    app.model_index - 1
                };
            }
        }
        Action::Select => match app.screen {
            Screen::Provider => {
                if let Some(p) = app.selected_provider() {
                    if p.id == CUSTOM_URL_ID {
                        app.begin_custom_url_input();
                    } else if !p.implemented {
                        app.flash(
                            format!("Provider `{}` is not yet implemented", p.display_name),
                            StatusKind::Warn,
                        );
                    } else {
                        app.screen = Screen::Model;
                        if app.models.is_empty() {
                            app.maybe_load_models_for_selection();
                        }
                    }
                }
            }
            Screen::Model => {
                // For "any model" providers, Enter pops a text input
                // so the user can type the model id. For providers
                // with a model list, Enter applies the highlighted
                // model directly.
                let is_any = matches!(
                    app.selected_provider().map(|p| &p.model_source),
                    Some(crate::provider::ModelSource::Any)
                );
                if is_any {
                    app.input_buffer.clear();
                    app.input_prompt = "Model id: ";
                    app.input_active = true;
                } else {
                    app.apply_selection();
                }
            }
            Screen::Status => {}
        },
        Action::BackToProvider => {
            app.screen = Screen::Provider;
        }
        Action::BackToModel => {
            app.screen = Screen::Model;
        }
        Action::GoStatus => {
            app.screen = Screen::Status;
        }
        Action::AddCustom => {
            app.begin_custom_url_input();
        }
        Action::RemoveCustom => {
            app.remove_current_custom();
        }
        Action::ToggleOnOff => {
            app.toggle();
        }
        Action::Verify => {
            app.verify();
        }
        Action::Refresh => {
            app.refresh();
        }
        Action::SubmitInput | Action::CancelInput | Action::Backspace | Action::Char(_) => {
            // Handled in input-mode branch above.
        }
        Action::Quit => unreachable!(),
    }
    true
}

/// Map a raw crossterm event to an Action, or None to ignore.
pub fn map_event(app: &App, event: CtEvent) -> Option<Action> {
    let CtEvent::Key(key) = event else {
        return None;
    };
    map_key(app, key)
}
