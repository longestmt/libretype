// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Code-editor routing based on the focused control's accessibility metadata.
//! Document contents and nearby conversations are not inspected.

pub fn supported(app: &str) -> bool {
    matches!(
        app,
        "com.microsoft.VSCode"
            | "com.microsoft.VSCodeInsiders"
            | "com.vscodium"
            | "com.todesktop.230313mzl4w4u92"
            | "com.exafunction.windsurf"
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Chat,
    Source,
    Other,
}

fn starts_label(value: &str, label: &str) -> bool {
    value.strip_prefix(label).is_some_and(|tail| {
        tail.is_empty()
            || tail.starts_with(|c: char| {
                c.is_whitespace() || matches!(c, ',' | ';' | ':' | '(' | '.')
            })
    })
}

pub fn classify(labels: &[String]) -> InputKind {
    // Labels are interface metadata, never AXValue. Do not infer chat from the
    // word "chat" in a source file or from a nearby sidebar's title.
    let labels: Vec<_> = labels
        .iter()
        .take(4)
        .filter(|s| s.len() <= 2048)
        .map(|s| s.trim().to_lowercase())
        .collect();
    if labels
        .iter()
        .any(|s| starts_label(s, "chat input") || starts_label(s, "inline chat input"))
    {
        InputKind::Chat
    } else if labels.iter().any(|s| starts_label(s, "editor content")) {
        InputKind::Source
    } else {
        InputKind::Other
    }
}

/// Class metadata is ordered from the focused control toward its ancestors.
/// A main editor must own the focused Monaco input, not merely be nearby.
pub fn classify_path(labels: &[String], path: &[Vec<String>]) -> InputKind {
    let direct = classify(labels);
    if direct != InputKind::Other {
        return direct;
    }
    let mut monaco = false;
    let input = path.first().is_some_and(|classes| {
        classes
            .iter()
            .any(|c| c == "inputarea" || c == "native-edit-context")
    });
    for classes in path.iter().take(12) {
        let has = |class: &str| classes.iter().take(64).any(|c| c == class);
        if has("xterm") || has("xterm-screen") {
            return InputKind::Other;
        }
        monaco |= has("monaco-editor");
        if has("interactive-input-part") && monaco && input {
            return InputKind::Chat;
        }
        if has("editor-instance") {
            return if monaco && input {
                InputKind::Source
            } else {
                InputKind::Other
            };
        }
    }
    InputKind::Other
}

pub fn automatic(kind: InputKind, source_enabled: bool) -> bool {
    kind == InputKind::Chat || (kind == InputKind::Source && source_enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modern_native_edit_contexts_keep_source_and_chat_scopes_separate() {
        let path = |parts: &[&str]| {
            parts
                .iter()
                .map(|s| vec![s.to_string()])
                .collect::<Vec<_>>()
        };
        let source = path(&[
            "native-edit-context",
            "overflow-guard",
            "monaco-editor",
            "editor-instance",
        ]);
        assert_eq!(
            classify_path(&["Untitled-1".into()], &source),
            InputKind::Source
        );
        let chat = path(&[
            "native-edit-context",
            "overflow-guard",
            "monaco-editor",
            "interactive-input-part",
        ]);
        assert_eq!(classify_path(&[], &chat), InputKind::Chat);
        for classes in [
            path(&[
                "ime-text-area",
                "overflow-guard",
                "monaco-editor",
                "editor-instance",
            ]),
            path(&["native-edit-context", "monaco-editor"]),
            path(&[
                "native-edit-context",
                "xterm",
                "monaco-editor",
                "editor-instance",
            ]),
        ] {
            assert_eq!(classify_path(&[], &classes), InputKind::Other);
        }
        assert!(supported("com.vscodium"));
        assert!(!supported("com.vscodium.fake"));
    }
    #[test]
    fn ancestor_classes_belong_to_the_focused_composer_not_a_neighbor() {
        let path = |parts: &[&str]| {
            parts
                .iter()
                .map(|s| s.split_whitespace().map(str::to_owned).collect())
                .collect::<Vec<_>>()
        };
        let source = path(&[
            "inputarea",
            "overflow-guard",
            "monaco-editor",
            "editor-instance",
        ]);
        assert_eq!(
            classify_path(&["notes.rs".into()], &source),
            InputKind::Source
        );
        let chat = path(&[
            "inputarea",
            "monaco-editor",
            "interactive-input-part",
            "editor-instance",
        ]);
        assert_eq!(classify_path(&["Entrée".into()], &chat), InputKind::Chat);
        assert_eq!(
            classify_path(&["Chat Input (Agent)".into()], &source),
            InputKind::Chat
        );
        for classes in [
            path(&["search-input", "monaco-editor", "editor-instance"]),
            path(&["inputarea", "monaco-editor"]),
            path(&["inputarea", "xterm", "editor-instance"]),
            path(&[
                "inputarea",
                "monaco-editor",
                "interactive-input-part-neighbor",
            ]),
        ] {
            assert_eq!(classify_path(&[], &classes), InputKind::Other);
        }
    }
    #[test]
    fn metadata_routing_distinguishes_chat_code_and_other_controls() {
        for label in [
            "Chat Input",
            "Chat Input (Agent). Press Enter to send out the request.",
            "Inline Chat Input, Use Alt+F1 for help.",
        ] {
            assert_eq!(classify(&[label.into()]), InputKind::Chat);
        }
        assert_eq!(
            classify(&["Editor content; Press Alt+F1 for accessibility options.".into()]),
            InputKind::Source
        );
        for label in [
            "chat.rs",
            "Search chat input",
            "Chat InputHistory",
            "Chat",
            "Terminal input",
            "Search",
            "",
        ] {
            assert_eq!(classify(&[label.into()]), InputKind::Other);
        }
        assert_eq!(classify(&["Chat Input".repeat(1000)]), InputKind::Other);
        assert!(automatic(InputKind::Chat, false));
        assert!(!automatic(InputKind::Source, false));
        assert!(automatic(InputKind::Source, true));
        assert!(!automatic(InputKind::Other, true));
        assert!(!supported("com.microsoft.VSCode.fake"));
        assert!(!supported("com.google.Chrome"));
    }
}
