# Agent Hook payload fixtures

These files preserve the Windows Hook payload shapes used to reproduce attribution integration behavior for Trae, CodeBuddy, and Qoder.

- All user identities, repositories, prompts, session identifiers, and transcript contents are synthetic.
- Windows paths intentionally retain a drive letter, backslashes, and a space in the user directory.
- Each file contains both `PreToolUse` and `PostToolUse` inputs.
- CodeBuddy contains separate CLI-style and IDE-style tool names. The fixture preserves both schemas so stage 3 tests can require identical file-edit and terminal classification behavior.
- Terminal payloads are parser fixtures only. Their commands are not executed by the fixture tests.

The wrapper fields (`agent`, `source`, and `cases`) are fixture metadata. Each `cases[].input` value is the JSON object supplied to the corresponding checkpoint preset.
