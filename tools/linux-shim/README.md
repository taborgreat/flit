# linux-shim

Lets the FLit plugin GUI (egui over X11/OpenGL) build on a Linux box that
has the X11 runtime libraries but not the `-dev` packages (no sudo needed).
Each `.pc` file points at a symlink in `lib/` to the system shared object.
`.cargo/config.toml` adds this directory to `PKG_CONFIG_PATH`.

With the dev packages installed (`libx11-xcb-dev libxcursor-dev
libxkbcommon-x11-dev libxcb-icccm4-dev libxcb-shape0-dev libxcb-xfixes0-dev
libgl-dev`) this shim is unnecessary and harmless. Windows and macOS never
use it.
