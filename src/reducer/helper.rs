use crate::state::EditorState;
// Helper function to mark editor as modified and save snapshot for undo
//
// ⭐ **The one choke point that knows the text changed.** A reparse costs O(document)
// (`lines.join("\n")` + a tree-sitter parse from scratch), so the highlighter is only
// told here — never for a wheel notch or a pointer move, which is why scrolling used to
// get slower as the file grew. Every edit path calls this before mutating.
pub fn mark_modified(editor: &mut EditorState) {
    editor.save_snapshot();
    editor.modified = true;
    editor.highlighter.mark_dirty();
}
