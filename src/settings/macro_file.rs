//! Portable macro files: one macro per JSON document, validated on import.

use serde::{Deserialize, Serialize};

use crate::models::Descriptor;

use super::state::{usage_for_key, Burst, FieldError, Macro, MacroAction, MacroActionKind};

pub const FORMAT: &str = "redragon-gui-macro";
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroFile {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub actions: Vec<MacroAction>,
    #[serde(default)]
    pub burst: Option<Burst>,
}

pub fn export_macro(macro_state: &Macro) -> String {
    let file = MacroFile {
        format: FORMAT.into(),
        version: VERSION,
        name: macro_state.name.clone(),
        actions: macro_state.actions.clone(),
        burst: macro_state.burst.clone(),
    };
    serde_json::to_string_pretty(&file).unwrap_or_else(|_| "{}".into())
}

/// Parse and validate a macro file against the model. Any unsupported value
/// rejects the whole file.
pub fn import_macro(desc: &Descriptor, text: &str) -> Result<MacroFile, Vec<FieldError>> {
    let file: MacroFile = match serde_json::from_str(text) {
        Ok(file) => file,
        Err(err) => {
            return Err(vec![FieldError {
                field: "file".into(),
                message: err.to_string(),
            }])
        }
    };
    let mut errors = Vec::new();
    if file.format != FORMAT {
        errors.push(FieldError {
            field: "format".into(),
            message: format!("expected `{FORMAT}`, found `{}`", file.format),
        });
    }
    if file.version != VERSION {
        errors.push(FieldError {
            field: "version".into(),
            message: format!("version {} is not supported", file.version),
        });
    }
    let Some(spec) = &desc.macros else {
        errors.push(FieldError {
            field: "macros".into(),
            message: "this model does not store macros".into(),
        });
        return Err(errors);
    };
    if file.actions.len() > spec.max_actions as usize {
        errors.push(FieldError {
            field: "actions".into(),
            message: format!(
                "{} actions exceed the {} this model stores",
                file.actions.len(),
                spec.max_actions
            ),
        });
    }
    for (index, action) in file.actions.iter().enumerate() {
        match action.kind {
            MacroActionKind::KeyDown | MacroActionKind::KeyUp => match action.key.as_deref() {
                None => errors.push(FieldError {
                    field: format!("actions[{index}]"),
                    message: "a key action needs a key".into(),
                }),
                Some(key) if usage_for_key(desc, key).is_none() => errors.push(FieldError {
                    field: format!("actions[{index}]"),
                    message: format!("`{key}` is not in this model's key table"),
                }),
                Some(_) => {}
            },
            MacroActionKind::Delay if action.delay_ms > 10_000 => errors.push(FieldError {
                field: format!("actions[{index}]"),
                message: "delays above 10 s cannot be encoded".into(),
            }),
            _ => {}
        }
    }
    if let Some(burst) = &file.burst {
        let Some(spec) = &spec.burst else {
            errors.push(FieldError {
                field: "burst".into(),
                message: "this model does not declare burst fire".into(),
            });
            return Err(errors);
        };
        if burst.bullets == 0 || burst.bullets > spec.max_bullets {
            errors.push(FieldError {
                field: "burst.bullets".into(),
                message: format!("expected 1..={}", spec.max_bullets),
            });
        }
        if burst.delay_ms < spec.min_delay_ms || burst.delay_ms > spec.max_delay_ms {
            errors.push(FieldError {
                field: "burst.delay_ms".into(),
                message: format!("expected {}..={} ms", spec.min_delay_ms, spec.max_delay_ms),
            });
        }
    }
    if errors.is_empty() {
        Ok(file)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::state::{seed_state, Macro};

    fn fixture() -> Descriptor {
        crate::models::fixture::descriptor()
    }

    #[test]
    fn macro_file_round_trips() {
        let desc = fixture();
        let state = seed_state(&desc);
        let mut macro_state = state.macros[0].clone();
        macro_state.name = "burst".into();
        macro_state.actions = vec![
            MacroAction {
                kind: MacroActionKind::KeyDown,
                key: Some("a".into()),
                button: None,
                delay_ms: 0,
            },
            MacroAction {
                kind: MacroActionKind::Delay,
                key: None,
                button: None,
                delay_ms: 40,
            },
            MacroAction {
                kind: MacroActionKind::KeyUp,
                key: Some("a".into()),
                button: None,
                delay_ms: 0,
            },
        ];
        macro_state.burst = Some(Burst {
            bullets: 3,
            delay_ms: 20,
        });
        let text = export_macro(&macro_state);
        let imported = import_macro(&desc, &text).expect("import");
        assert_eq!(imported.name, "burst");
        assert_eq!(imported.actions, macro_state.actions);
        assert_eq!(imported.burst, macro_state.burst);
    }

    #[test]
    fn unsupported_action_rejects_the_file() {
        let desc = fixture();
        let macro_state = Macro {
            slot: 0,
            name: "bad".into(),
            actions: vec![MacroAction {
                kind: MacroActionKind::KeyDown,
                key: Some("nope".into()),
                button: None,
                delay_ms: 0,
            }],
            burst: None,
        };
        let text = export_macro(&macro_state);
        let errors = import_macro(&desc, &text).expect_err("unsupported key");
        assert!(errors[0].message.contains("key table"), "{errors:?}");
    }

    #[test]
    fn oversized_action_list_is_rejected() {
        let desc = fixture();
        let actions = vec![
            MacroAction {
                kind: MacroActionKind::Delay,
                key: None,
                button: None,
                delay_ms: 1,
            };
            desc.macros.as_ref().unwrap().max_actions as usize + 1
        ];
        let macro_state = Macro {
            slot: 0,
            name: "long".into(),
            actions,
            burst: None,
        };
        let errors = import_macro(&desc, &export_macro(&macro_state)).expect_err("too long");
        assert!(errors.iter().any(|e| e.field == "actions"), "{errors:?}");
    }
}
