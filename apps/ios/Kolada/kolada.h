// The C interface of crates/kolada-ios (src/lib.rs says what each call does).
// Call on the main thread; `wake` may come from any thread.
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct KoladaApp KoladaApp;

// What iOS answers the model with (crates/kolada-ios/src/services.rs says
// what each does). Given once, before the first kolada_open; any may be null.
typedef void (*KoladaSink)(void *to, const void *bytes, size_t length);
typedef struct KoladaServices {
    size_t (*spell_check)(const uint16_t *units, size_t length, size_t *out, size_t most);
    void (*spell_learn)(const char *word);
    void (*spell_guesses)(const char *word, KoladaSink sink, void *to);
    uint16_t (*web)(const char *method, const char *url, const char *headers, const uint8_t *body, size_t body_length, size_t most, KoladaSink sink, void *to);
    bool (*picture_size)(const char *path, const uint8_t *bytes, size_t length, uint32_t *width, uint32_t *height);
    bool (*picture_draw)(const char *path, const uint8_t *bytes, size_t length, uint32_t width, uint32_t height, uint8_t *bgra);
    bool (*picture_encode)(const uint8_t *bgra, uint32_t width, uint32_t height, uint8_t quality, bool png, KoladaSink sink, void *to);
    bool (*picture_text)(const char *path, KoladaSink sink, void *to);
    void (*clock)(int64_t seconds, KoladaSink sink, void *to);
    void (*font_file)(const char *name, KoladaSink sink, void *to);
    bool (*color_glyph)(const char *name, uint16_t glyph, float size, int32_t *left, int32_t *top, uint32_t *width, uint32_t *height, KoladaSink sink, void *to);
    bool (*recycle)(const char *path);
    bool (*app_installed)(const char *name);
    const char *fonts_dir;
} KoladaServices;
void kolada_services(const KoladaServices *services);

KoladaApp *kolada_open(const char *vault, bool dark);
void kolada_close(KoladaApp *app);
void kolada_start(KoladaApp *app, void (*wake)(void *context), void *context);
bool kolada_poll(KoladaApp *app);

void kolada_frame(KoladaApp *app, uint32_t width, uint32_t height, float scale);
const uint32_t *kolada_pixels(KoladaApp *app);
bool kolada_wants_frames(KoladaApp *app);
double kolada_next_due(KoladaApp *app);
bool kolada_tick(KoladaApp *app);
void kolada_set_dark(KoladaApp *app, bool dark);
void kolada_reduce_motion(KoladaApp *app, bool reduce);

bool kolada_insert(KoladaApp *app, const char *text);
bool kolada_command(KoladaApp *app, const char *name);
bool kolada_do(KoladaApp *app, const char *what);

bool kolada_pointer_down(KoladaApp *app, float x, float y, uint32_t clicks, bool shift);
bool kolada_pointer_dragged(KoladaApp *app, float x, float y);
bool kolada_pointer_up(KoladaApp *app, float x, float y);
bool kolada_tap(KoladaApp *app, float x, float y, uint32_t clicks);
char *kolada_take_event(KoladaApp *app);
char *kolada_selected_text(KoladaApp *app);
bool kolada_paste(KoladaApp *app, const char *plain, const char *html, const char *markdown);
bool kolada_paste_plain(KoladaApp *app, const char *plain);
bool kolada_import_files(KoladaApp *app, const char *paths);
bool kolada_import_folder(KoladaApp *app, const char *folder);
bool kolada_import_clipboard(KoladaApp *app, const char *text);
bool kolada_switch_vault(KoladaApp *app, const char *folder);
char *kolada_export(KoladaApp *app, const char *kind);
bool kolada_pointer_moved(KoladaApp *app, float x, float y);
bool kolada_pointer_left(KoladaApp *app);
bool kolada_scroll(KoladaApp *app, float dy);

void kolada_selection(KoladaApp *app, size_t *location, size_t *length);
void kolada_select(KoladaApp *app, size_t location, size_t length);
bool kolada_marked(KoladaApp *app, size_t *location, size_t *length);
bool kolada_set_marked(KoladaApp *app, const char *text, size_t cursor, size_t converting);
bool kolada_unmark(KoladaApp *app);
size_t kolada_length(KoladaApp *app);
char *kolada_substring(KoladaApp *app, size_t location, size_t length);
void kolada_free_string(char *s);
bool kolada_caret(KoladaApp *app, float *rect);
bool kolada_rect(KoladaApp *app, size_t location, size_t length, float *rect);
size_t kolada_line_of(KoladaApp *app, size_t at);
bool kolada_line_range(KoladaApp *app, size_t line, size_t *location, size_t *length);
void kolada_visible(KoladaApp *app, size_t *location, size_t *length);
char *kolada_label(KoladaApp *app);
size_t kolada_index_at(KoladaApp *app, float x, float y);

// Spelling, damage, links (lib.rs).
bool kolada_spell_step(KoladaApp *app, bool *more);
int64_t kolada_damage(KoladaApp *app, int32_t *rects, size_t most);
bool kolada_link_click(KoladaApp *app, float x, float y);
// The shortcuts and the menu bar (keys.rs): "menu\ttitle\tkey\tmods\tcommand" lines.
char *kolada_commands(void);
// Menus (menus.rs): "label\tkey\tdivider" lines; the index picked back.
uint8_t kolada_press_kind(KoladaApp *app, float x, float y);
char *kolada_menu(KoladaApp *app, float x, float y, bool has_html);
char *kolada_menu_requested(KoladaApp *app);
bool kolada_menu_pick(KoladaApp *app, size_t index);
// The pasteboard (clip.rs).
bool kolada_copy(KoladaApp *app, char **plain, char **html, char **markdown);
bool kolada_cut(KoladaApp *app, char **plain, char **html, char **markdown);
bool kolada_paste_picture(KoladaApp *app, const uint8_t *png, size_t length);
// Files coming in (arrive.rs).
bool kolada_drag_enter(KoladaApp *app, const char *names, float x, float y);
bool kolada_drag_over(KoladaApp *app, float x, float y);
bool kolada_drag_leave(KoladaApp *app);
int64_t kolada_drop(KoladaApp *app);
bool kolada_drop_files(KoladaApp *app, const char *paths, float x, float y, int64_t end);
bool kolada_open_file(KoladaApp *app, const char *path);
// The vault is looked at for other apps' changes only while in front (watch.rs).
void kolada_set_active(KoladaApp *app, bool active);
