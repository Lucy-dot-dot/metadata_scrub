#!/usr/bin/env fish

set -l app_name metadata_scrub
set -l repo_dir (status dirname)
set -l desktop_file "$repo_dir/$app_name.desktop"
set -l bin "$HOME/.cargo/bin/$app_name"

if not test -x "$bin"
    echo "error: $bin not found - run 'cargo install --path .' first" >&2
    exit 1
end
if not test -f "$desktop_file"
    echo "error: $desktop_file not found" >&2
    exit 1
end

# Icon: install to hicolor theme if icon.svg or icon.png exists in the repo
if test -f "$repo_dir/assets/icon.svg"
    mkdir -p ~/.local/share/icons/hicolor/scalable/apps
    cp "$repo_dir/assets/icon.svg" ~/.local/share/icons/hicolor/scalable/apps/$app_name.svg
    echo "icon installed: ~/.local/share/icons/hicolor/scalable/apps/$app_name.svg"
    # pre-rendered sizes so small icons are crisp instead of downscaled from SVG
    for size in 16 24 32 48 64 128 256
        set -l png "$repo_dir/assets/icons/icon_$size.png"
        if test -f "$png"
            mkdir -p ~/.local/share/icons/hicolor/"$size"x"$size"/apps
            cp "$png" ~/.local/share/icons/hicolor/"$size"x"$size"/apps/$app_name.png
        end
    end
else if test -f "$repo_dir/assets/icon_256.png"
    mkdir -p ~/.local/share/icons/hicolor/256x256/apps
    cp "$repo_dir/assets/icon_256.png" ~/.local/share/icons/hicolor/256x256/apps/$app_name.png
    echo "icon installed: ~/.local/share/icons/hicolor/256x256/apps/$app_name.png"
else
    echo "note: no assets/icon.svg or assets/icon_256.png, skipping icon install"
end

# Refresh icon theme cache
if type -q gtk-update-icon-cache
    gtk-update-icon-cache -f -t ~/.local/share/icons/hicolor >/dev/null 2>&1
end

# App launcher entry
mkdir -p ~/.local/share/applications
cp "$desktop_file" ~/.local/share/applications/
echo "installed: ~/.local/share/applications/$app_name.desktop"

# Desktop shortcut
if test -d ~/Desktop
    cp "$desktop_file" ~/Desktop/
    chmod +x ~/Desktop/$app_name.desktop
    echo "installed: ~/Desktop/$app_name.desktop"
end

# Refresh KDE caches
if type -q kbuildsycoca6
    kbuildsycoca6 --noincremental >/dev/null 2>&1
else if type -q kbuildsycoca5
    kbuildsycoca5 --noincremental >/dev/null 2>&1
end

echo "done"
