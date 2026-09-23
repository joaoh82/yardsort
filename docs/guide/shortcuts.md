# Keyboard shortcuts

**Mod** is `⌘` on macOS and **`Ctrl+Shift`** on Linux and Windows.

| Shortcut      | Linux / Windows      | macOS       | Action                                    |
| ------------- | -------------------- | ----------- | ----------------------------------------- |
| Mod+O         | `Ctrl+Shift+O`       | `⌘O`        | Open a project (folder picker)            |
| Mod+N         | `Ctrl+Shift+N`       | `⌘N`        | New workspace in the current project      |
| Mod+T         | `Ctrl+Shift+T`       | `⌘T`        | New shell tab in the selected workspace   |
| Mod+W         | `Ctrl+Shift+W`       | `⌘W`        | Close the active tab                      |
| Mod+B         | `Ctrl+Shift+B`       | `⌘B`        | Toggle the left panel                     |
| Mod+Alt+B     | `Ctrl+Shift+Alt+B`   | `⌥⌘B`       | Toggle the right panel                    |
| Mod+,         | `Ctrl+Shift+,`       | `⌘,`        | Settings                                  |
| Mod+C / Mod+V | `Ctrl+Shift+C` / `V` | `⌘C` / `⌘V` | Copy the selection / paste, in a terminal |

In the composer: `Enter` starts, `Shift+Enter` adds a line, `Esc` cancels. `Esc` also closes
dialogs and menus. In an agent's terminal `Shift+Enter` adds a line too — see
[Typing to an agent](terminals-and-sessions.md#typing-to-an-agent).

## Why Ctrl+Shift?

Because plain `Ctrl`+letter belongs to the program in the terminal. `Ctrl+B` moves back a
character in a shell and is the tmux prefix; `Ctrl+W` deletes a word; `Ctrl+T` transposes;
`Ctrl+C` interrupts. An app that takes those away makes its terminal worse than a real one.
`Ctrl+Shift` is what terminal emulators have always used for their own commands, so Yardsort
does too. On macOS `⌘` never reaches the terminal, so it is free to use.

Shortcuts follow the character your keyboard layout produces, not the physical key, so they match
the key caps on Dvorak, AZERTY and friends.
