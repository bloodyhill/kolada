// The C interface of crates/notsy-ios (src/lib.rs says what each call does).
// Call on the main thread; `wake` may come from any thread.
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct NotsyApp NotsyApp;

NotsyApp *notsy_open(const char *vault, bool dark);
void notsy_close(NotsyApp *app);
void notsy_start(NotsyApp *app, void (*wake)(void *context), void *context);
bool notsy_poll(NotsyApp *app);

void notsy_frame(NotsyApp *app, uint32_t width, uint32_t height, float scale);
const uint32_t *notsy_pixels(NotsyApp *app);
bool notsy_wants_frames(NotsyApp *app);
double notsy_next_due(NotsyApp *app);
bool notsy_tick(NotsyApp *app);
void notsy_set_dark(NotsyApp *app, bool dark);
void notsy_reduce_motion(NotsyApp *app, bool reduce);

bool notsy_insert(NotsyApp *app, const char *text);
bool notsy_command(NotsyApp *app, const char *name);
bool notsy_do(NotsyApp *app, const char *what);

bool notsy_pointer_down(NotsyApp *app, float x, float y, uint32_t clicks, bool shift);
bool notsy_pointer_dragged(NotsyApp *app, float x, float y);
bool notsy_pointer_up(NotsyApp *app, float x, float y);
bool notsy_tap(NotsyApp *app, float x, float y, uint32_t clicks);
char *notsy_take_event(NotsyApp *app);
char *notsy_selected_text(NotsyApp *app);
bool notsy_paste(NotsyApp *app, const char *plain, const char *html);
bool notsy_paste_plain(NotsyApp *app, const char *plain);
bool notsy_import_files(NotsyApp *app, const char *paths);
bool notsy_import_folder(NotsyApp *app, const char *folder);
bool notsy_import_clipboard(NotsyApp *app, const char *text);
bool notsy_switch_vault(NotsyApp *app, const char *folder);
char *notsy_export(NotsyApp *app, const char *kind);
bool notsy_pointer_moved(NotsyApp *app, float x, float y);
bool notsy_pointer_left(NotsyApp *app);
bool notsy_scroll(NotsyApp *app, float dy);

void notsy_selection(NotsyApp *app, size_t *location, size_t *length);
void notsy_select(NotsyApp *app, size_t location, size_t length);
bool notsy_marked(NotsyApp *app, size_t *location, size_t *length);
bool notsy_set_marked(NotsyApp *app, const char *text, size_t cursor, size_t converting);
bool notsy_unmark(NotsyApp *app);
size_t notsy_length(NotsyApp *app);
char *notsy_substring(NotsyApp *app, size_t location, size_t length);
void notsy_free_string(char *s);
bool notsy_caret(NotsyApp *app, float *rect);
bool notsy_rect(NotsyApp *app, size_t location, size_t length, float *rect);
size_t notsy_line_of(NotsyApp *app, size_t at);
bool notsy_line_range(NotsyApp *app, size_t line, size_t *location, size_t *length);
void notsy_visible(NotsyApp *app, size_t *location, size_t *length);
char *notsy_label(NotsyApp *app);
size_t notsy_index_at(NotsyApp *app, float x, float y);
