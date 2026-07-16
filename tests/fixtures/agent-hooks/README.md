# Agent Hook payload fixtures

These files preserve the Windows Hook payload shapes used to reproduce attribution integration behavior for Trae, CodeBuddy, and Qoder.

- All user identities, repositories, prompts, session identifiers, and transcript contents are synthetic.
- Windows paths intentionally retain a drive letter, backslashes, and a space in the user directory.
- Each file contains both `PreToolUse` and `PostToolUse` inputs.
- CodeBuddy contains separate CLI-style and IDE-style tool names. During stage 0, the IDE aliases intentionally reproduce the existing `ToolClass::Skip` gap; the stage 3 fix must update both the implementation and those expectations.
- Terminal payloads are parser fixtures only. Their commands are not executed by the fixture tests.

The wrapper fields (`agent`, `source`, and `cases`) are fixture metadata. Each `cases[].input` value is the JSON object supplied to the corresponding checkpoint preset.
