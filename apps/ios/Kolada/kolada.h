// The C interface of crates/kolada-ios (src/lib.rs says what each call does).
// Call on the main thread; `wake` may come from any thread.
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct KoladaApp KoladaApp;

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
bool kolada_paste(KoladaApp *app, const char *plain, const char *html);
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
