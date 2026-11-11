# 39 — The desktop app's sizes

_Proposed in Figma and built on 2026-11-10, after feedback that the app felt very small. Checked in
a headless browser against the real components; not yet looked at in the app's own window on the
three systems, and the screenshots in `docs/images/` are not retaken._

The app's base text was 13 px, but most of its text was smaller: branch names, statuses, section
labels, badges, hints and both bottom bars were 10 or 11 px, on rows 28 px tall, in a faint colour
with a contrast of 3.7:1 against the panels. This makes everything one step larger, and the
smallest text most. It changes sizes and one colour. No control is added, removed, moved or
renamed.

The design is in Figma, in the Yardsort project:
[Yardsort desktop: larger UI proposal](https://www.figma.com/design/HXSYFYmKvlzr1IXp0hBEh3). It
draws the main window three times from one set of variables, as modes: Current, Proposed and
Larger. **Proposed** is what is built. Names in it are made up.

## The scale

Text sizes are four named sizes in `src/styles.css`, used as `text-ui`, `text-detail`, `text-small`
and `text-micro`. No text is smaller than `micro`. Headings keep Tailwind's own sizes, `text-base`
and up.

| Size     | Was | Is  | Where                                                         |
| -------- | --- | --- | ------------------------------------------------------------- |
| `ui`     | 13  | 14  | Row labels, tab titles, buttons, body text                    |
| `detail` | 12  | 13  | Hints and secondary lines in dialogs and views, inline code   |
| `small`  | 11  | 12  | Branch names, statuses, section labels, the two bottom bars   |
| `micro`  | 10  | 11  | Count badges, pull request numbers, key hints                 |
| Code     | 12  | 13  | Diffs, the file viewer, the workflow editor                   |
| Terminal | 13  | 13  | Unchanged: a normal terminal size, and columns are worth more |

Heights and targets moved one Tailwind step, 4 px, each:

| What                                       | Was | Is  | Class    |
| ------------------------------------------ | --- | --- | -------- |
| Status bar, workspace footer, file tree    | 24  | 28  | `h-7`    |
| Rows and compact controls                  | 28  | 32  | `h-8`    |
| Form controls, the viewer's header         | 32  | 36  | `h-9`    |
| Panel headers, sidebar nav rows, tab strip | 36  | 40  | `h-10`   |
| Icon buttons                               | 24  | 28  | `size-7` |
| Icons                                      | 14  | 16  | `size-4` |
| Agent icon in a tab or a list              | 12  | 14  |          |
| Status dots                                | 6   | 8   | `size-2` |
| Count badges                               | 16  | 18  |          |

Rows and headers have 14 px at their sides, where they had 12, and a workspace sits 32 px in under
its project, where it sat 28. The side panels open 280 and 368 px wide in a new profile, where
they opened 260 and 340, so names are cut off where they were. A profile that has been used keeps
the widths it saved.

Faint text is `#808a99` in dark, 5.1:1 against the panels, where it was `#6b7380`; and `#667080`
in light, 5.0:1, where it was `#7b8493`, 3.8:1.

## What it costs

In the default window, 1440 × 900, measured on the drawing: the terminal has about 98 columns where
it had about 104, and one line fewer; a Projects list with three workflows above it shows 13 rows
where it showed 17.

## Where it differs from the Figma file

- **Buttons sized by their padding**, not by a fixed height, come out within a few pixels of the
  drawing and not on it: Commit is 33 px tall where the file draws 32.
- **`detail`, the 13 px size, is not in the file.** The main window has almost none of it. It is
  the step between `ui` and `small`, as 12 px was between 13 and 11.
- **Only the main window is drawn, and only in dark.** Dialogs, Settings, and the Pull requests,
  Tasks, Tests, Schedules, Triggers and Usage views follow the same scale without a drawing. So
  does the light mode's faint colour.

## To do

- Look at it in the app's own window on Linux, macOS and Windows, with the window at its minimum
  width, 900: the two side panels now take 648 px of it when both are open at their defaults.
- Retake the screenshots in `docs/images/`.
