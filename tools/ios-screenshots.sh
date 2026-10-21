#!/usr/bin/env bash
# The App Store's screenshots, from sample notes written here (never anyone's
# own): iPad 13-inch at 2064 x 2752 and iPhone 6.9-inch at 1320 x 2868, the
# sizes App Store Connect asks for, the status bar at 9:41. Driven by
# Driver.swift (tools/ios-drive.sh); each picture is the simulator's whole
# screen (xcrun simctl io), taken when the script signals for it.
#
#   tools/ios-screenshots.sh        pictures in target/ios/screenshots/
set -euo pipefail
cd "$(dirname "$0")/.."
out=target/ios/screenshots
work=target/ios/drive/screenshots
rm -rf "$out" "$work"
mkdir -p "$out" "$work/vault/Kitchen"
tools/ios-build.sh >/dev/null

v=$work/vault
cp "assets/welcome/Welcome to Kolada.png" "$v/bird.png"
cat > "$v/Garden plan.md" <<'EOF'
# Garden plan

Tomatoes go along the south fence this year, with **basil** between them.

- [ ] Order seeds @fri
- [ ] Fix the water barrel
- [x] Turn the compost

| Bed | Crop | Sown |
| --- | --- | --- |
| 1 | Tomatoes | April |
| 2 | Beans | May |

See [[Recipes]] for what to do with them all.

![[bird.png]]
EOF
cat > "$v/Kitchen/Recipes.md" <<'EOF'
# Recipes

## Tomato soup

1. Roast the tomatoes with garlic and a little oil.
2. Blend with stock until smooth.
3. Finish with ==fresh basil== and black pepper.

> Taste before you salt: the stock already has some.

## Green beans

- Blanch for three minutes
- Toss with lemon and toasted almonds
EOF
cat > "$v/Trip to Lisbon.md" <<'EOF'
# Trip to Lisbon

- [ ] Book the train to Sintra @today 4pm
- [ ] Renew the passport @in 3 days
- [x] Ask Ana about the tram pass

Pack light: it is warm until late October.
EOF
cat > "$v/Reading list.md" <<'EOF'
# Reading list

- The Overstory
- A Pattern Language
- Braiding Sweetgrass

#books
EOF
touch "$v/Garden plan.md"

shoot() { # device, script, names...
    local device=$1 script=$2
    shift 2
    local udid
    udid=$(xcrun simctl list devices available -j | python3 -c "import json,sys; d=json.load(sys.stdin)['devices']; print(next(x['udid'] for r in d.values() for x in r if x['name']==sys.argv[1]))" "$device")
    xcrun simctl boot "$udid" 2>/dev/null || true
    xcrun simctl bootstatus "$udid" -b >/dev/null
    xcrun simctl ui "$udid" appearance light
    xcrun simctl status_bar "$udid" override --time 9:41 --dataNetwork wifi --wifiMode active --wifiBars 3 --cellularMode active --cellularBars 4 --batteryState charged --batteryLevel 100
    bash tools/ios-drive.sh "$script" --device "$device" --fresh --vault "$v" --no-build --wait 2 >/dev/null || true
    for name in "$@"; do
        for _ in $(seq 1 600); do grep -q "signal $name" "$work/drive.log" 2>/dev/null && break; sleep 0.1; done
        sleep 0.4
        xcrun simctl io "$udid" screenshot "$out/$name.png" >/dev/null 2>&1
        echo "$name: $(sips -g pixelWidth -g pixelHeight "$out/$name.png" | awk '/pixel/ {printf "%s ", $2}')"
    done
    for _ in $(seq 1 300); do grep -q '^done$' "$work/drive.log" 2>/dev/null && break; sleep 0.1; done
    xcrun simctl status_bar "$udid" clear
    xcrun simctl terminate "$udid" app.kolada.ios 2>/dev/null || true
}

cat > "$work/ipad.txt" <<'EOF'
wait 2
resign
wait 1
signal ipad-1-garden
wait 2
key cmd+o
type tomato
wait 1.5
resign
signal ipad-2-search
wait 2
do escape
target Row(0)
wait 0.5
target Row(1)
wait 1
dark on
resign
wait 1
signal ipad-3-recipes-dark
wait 2
key cmd+t
wait 1.5
signal ipad-4-today-dark
wait 2
EOF
shoot "iPad Pro 13-inch (M5)" "$work/ipad.txt" ipad-1-garden ipad-2-search ipad-3-recipes-dark ipad-4-today-dark

cat > "$work/iphone.txt" <<'EOF'
wait 2
resign
wait 1
signal iphone-1-garden
wait 2
edge
wait 1
signal iphone-2-menu
wait 2
do escape
dark on
key cmd+t
wait 1.5
resign
signal iphone-3-today-dark
wait 2
EOF
shoot "iPhone 18 Pro Max" "$work/iphone.txt" iphone-1-garden iphone-2-menu iphone-3-today-dark
echo "in $out"
