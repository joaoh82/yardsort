# What the terminal in the page is drawn with

[xterm.js](https://github.com/xtermjs/xterm.js), the terminal the Yardsort app itself uses, MIT
licensed (`LICENSE-xterm`, `LICENSE-addon-fit`), unmodified:

| File           | From                                           |
| -------------- | ---------------------------------------------- |
| `xterm.js`     | `@xterm/xterm` 6.0.0, `lib/xterm.js`           |
| `xterm.css`    | `@xterm/xterm` 6.0.0, `css/xterm.css`          |
| `addon-fit.js` | `@xterm/addon-fit` 0.11.0, `lib/addon-fit.js`  |

The dashboard has no build step and is compiled into `yardsort-host` and the app, which are built
where there is no `node_modules`, so the files are kept here. They are the ones the root
`package.json` installs: a test (`the_terminal_library_kept_here_is_the_one_the_app_installs`)
fails when they differ. To move to a newer release, change the app's first, then copy the three
files again and change the versions above.
