# The Mac's updater

Built, as Windows' updater is (`kolada-win` `updates.rs`): the same rules in `kolada_core::update`, the same pill above the side menu and the same Settings switch, "Check for updates". The earlier plan here (an Ed25519-signed feed, a three-way Automatic / Ask / Off row) was not built: the Mac follows what Windows ships, and its download is checked by Apple's own signature instead.

## Which copies update themselves

Only the website's build: a `Kolada.app` bundle signed with Kolada's Developer ID (`codesign -R`, team `5GQ2248AFB`), in a folder the person may write (`/Applications` for an admin, else wherever they keep it), not under a cargo `target` folder. A test instance updates only from a feed named by `KOLADA_UPDATE_FEED` (a server on this machine). Finding this out runs `codesign`, so it happens on a thread after the first frame (`updates::find_out`); Settings shows the switch from then on (`Model::set_updater`).

## The feed

`https://kolada.app/download/mac.json` (`update::FEED_MAC`), the same JSON as Windows':

    { "version": "0.4.0", "url": "https://kolada.app/download/Kolada-0.4.0-mac.zip", "sha256": "<64 hex>", "size": 3100000 }

The zip is the notarized, stapled app as `ditto -c -k --keepParent` makes it. `tools/mac-release.sh` writes both, `target/Kolada-<version>-mac.zip` and `target/mac.json`, beside the DMG. The website serves them at `/download/`; the download must be on the feed's own site, over HTTPS, and say its hash and size (`Feed::parse`).

## How

1. Twenty seconds after the first frame, a thread at background priority asks the feed, at most once a day (`update::due`), with Foundation's loading (`web.rs`). Offline, it asks again within the hour.
2. A newer version: the zip is downloaded and checked against the feed's SHA-256 and size, unpacked with `ditto` into the app data's `update/unpacked`, and the app inside must be signed by Kolada's Developer ID and pass Gatekeeper (`spctl --assess`, i.e. notarized). Anything that fails is deleted.
3. It is staged as `update/Kolada.app` with a `ready` note of its version (`update::stage_bundle`), and the pill shows ("Update ready · 0.4.0", "Restart").
4. The pill: the note and session are saved, the running bundle is renamed aside to `.Kolada.old.app` (macOS lets a running app's folder be renamed) and the new one renamed into its place (`update::swap_bundle`; a copy with `ditto` first when they are on different volumes). The app quits, and a small shell waits for it to end and opens the new one with `--updated`, on the same note (the session).
5. Never clicked: at the next start, before the window, the same swap, then the new app opens in this one's place (`updates::install_waiting`: one look at a file when nothing waits).
6. After the first frame the new one removes `.Kolada.old.app`.

## Tests

`kolada-core` `a_mac_bundle_waits_staged_until_it_is_newer_than_the_app`, `the_new_bundle_takes_the_running_ones_place`; `kolada-app` `a_copy_that_updates_itself_offers_the_switch_and_its_pill_asks_the_shell_to_put_the_update_in`; `kolada-mac` `a_bundle_not_signed_by_kolada_is_not_unpacked`, `a_test_binary_is_no_bundle_and_never_updates_itself`.

## Releasing

- Upload `Kolada-<version>-mac.zip` and `mac.json` to `kolada.app/download/` with each Mac release (the zip within the site's 25 MiB a file). Until `mac.json` is there, the check finds nothing and says nothing.
- Copies from before this (0.1.x, 0.2.0) have no updater: they need the new DMG once.
