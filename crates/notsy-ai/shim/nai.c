// A small C face on llama.cpp, so Rust never mirrors its structs: open a
// model, run one prompt to one answer, close.

#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "llama.h"

// The model's state after a prompt's fixed opening (instructions and
// examples), so only what follows it is read again.
typedef struct nai_opening {
    char *text;
    int len;
    int n_tokens;
    uint8_t *state;
    size_t size;
} nai_opening;

#define NAI_PREFIXES 8

typedef struct nai {
    struct llama_model *model;
    struct llama_context *ctx;
    const struct llama_vocab *vocab;
    const volatile int *cancel;
    llama_token_data *all;
    int n_vocab;
    int n_batch;
    llama_token end;
    nai_opening kept[NAI_PREFIXES];
    int next_kept;
    // The bytes of the next prompt that are a fixed opening.
    int keep;
    // The text the next run should copy unless it is sure of a change.
    llama_token *src;
    int n_src;
    float near;
    float far;
    // The last run, for the log and the eval.
    int n_prompt;
    int n_cached;
    int n_out;
    int n_kept;
    int n_left;
    int64_t prefill_us;
    int64_t decode_us;
} nai;

typedef struct nai_stats {
    int n_prompt;
    // Of those, restored rather than read.
    int n_cached;
    int n_out;
    // Tokens the anchor held against the model's choice, and times the
    // answer left it anyway.
    int n_kept;
    int n_left;
    double prefill_ms;
    double decode_ms;
} nai_stats;

enum {
    NAI_OK = 0,
    NAI_CANCELLED = 1,
    NAI_LONG = 2,
    NAI_TOO_BIG = -2,
    NAI_BAD_GRAMMAR = -3,
    NAI_FAILED = -4,
};

static FILE *nai_log_file;
static enum ggml_log_level nai_log_last = GGML_LOG_LEVEL_INFO;

static void quiet(enum ggml_log_level level, const char *text, void *data) {
    (void)level;
    (void)text;
    (void)data;
}

// Debug lines (one per graph built) would swamp a log kept for a session.
static void to_file(enum ggml_log_level level, const char *text, void *data) {
    (void)data;
    if (level != GGML_LOG_LEVEL_CONT) {
        nai_log_last = level;
    }
    if (nai_log_last == GGML_LOG_LEVEL_DEBUG) {
        return;
    }
    fputs(text, nai_log_file);
    fflush(nai_log_file);
}

// llama.cpp says nothing unless `path` (UTF-8) names a log to append to.
void nai_log(const char *path) {
    if (path && *path && !nai_log_file) {
        nai_log_file = ggml_fopen(path, "a");
    }
    llama_log_set(nai_log_file ? to_file : quiet, NULL);
}

void nai_note(const char *text, int len) {
    if (nai_log_file) {
        fwrite(text, 1, (size_t)len, nai_log_file);
        fflush(nai_log_file);
    }
}

// Every choice the anchor weighs goes to the log too, for tuning.
static bool nai_tracing;

void nai_trace(int on) {
    nai_tracing = on != 0;
}

static bool stop(void *data) {
    const nai *h = data;
    return h->cancel && *h->cancel;
}

nai *nai_open(const char *path, int threads, int threads_batch, int n_ctx) {
    if (!nai_log_file) {
        llama_log_set(quiet, NULL);
    }
    llama_backend_init();

    struct llama_model_params mp = llama_model_default_params();
    mp.n_gpu_layers = 0;
    // Mapped, so every Notsy process shares one copy of the weights.
    mp.load_mode = LLAMA_LOAD_MODE_MMAP;
    // Repacked weights would be a private copy per process.
    mp.use_extra_bufts = false;
    struct llama_model *model = llama_model_load_from_file(path, mp);
    if (!model) {
        return NULL;
    }

    nai *h = calloc(1, sizeof *h);
    if (!h) {
        llama_model_free(model);
        return NULL;
    }
    struct llama_context_params cp = llama_context_default_params();
    cp.n_ctx = (uint32_t)n_ctx;
    // A paragraph fits a batch; a smaller one keeps the compute buffer small.
    cp.n_batch = 256;
    cp.n_ubatch = 256;
    cp.n_seq_max = 1;
    // Only the last token's logits are ever read: a row for each token of a
    // batch, of a 65k vocabulary, would hold 64 MB for nothing.
    cp.n_outputs_max = 1;
    cp.n_threads = threads;
    cp.n_threads_batch = threads_batch;
    cp.no_perf = true;
    cp.abort_callback = stop;
    cp.abort_callback_data = h;
    struct llama_context *ctx = llama_init_from_model(model, cp);
    if (!ctx) {
        free(h);
        llama_model_free(model);
        return NULL;
    }
    h->model = model;
    h->ctx = ctx;
    h->vocab = llama_model_get_vocab(model);
    h->n_vocab = llama_vocab_n_tokens(h->vocab);
    h->n_batch = (int)llama_n_batch(ctx);
    h->end = llama_vocab_eot(h->vocab);
    if (h->end == LLAMA_TOKEN_NULL) {
        h->end = llama_vocab_eos(h->vocab);
    }
    h->all = malloc(sizeof *h->all * (size_t)h->n_vocab);
    if (!h->all) {
        llama_free(ctx);
        llama_model_free(model);
        free(h);
        return NULL;
    }
    return h;
}

void nai_close(nai *h) {
    if (!h) {
        return;
    }
    free(h->all);
    free(h->src);
    for (int i = 0; i < NAI_PREFIXES; i++) {
        free(h->kept[i].text);
        free(h->kept[i].state);
    }
    llama_free(h->ctx);
    llama_model_free(h->model);
    free(h);
    llama_backend_free();
}

int nai_n_ctx(const nai *h) {
    return (int)llama_n_ctx(h->ctx);
}

const char *nai_template(const nai *h) {
    return llama_model_chat_template(h->model, NULL);
}

// A conversation in the model's own chat format, `n` turns of `roles` and
// `texts`, open for the model's answer when `open`, and without the BOS the
// vocab adds when it is read. The length written, or -1 if the model's
// template is not one llama.cpp knows or `cap` is too small.
int nai_chat(const nai *h, const char **roles, const char **texts, int n, int open, char *out, int cap) {
    const char *tmpl = llama_model_chat_template(h->model, NULL);
    if (!tmpl || n <= 0) {
        return -1;
    }
    struct llama_chat_message *msgs = malloc(sizeof *msgs * (size_t)n);
    if (!msgs) {
        return -1;
    }
    for (int i = 0; i < n; i++) {
        msgs[i].role = roles[i];
        msgs[i].content = texts[i];
    }
    int len = llama_chat_apply_template(tmpl, msgs, (size_t)n, open != 0, out, cap);
    free(msgs);
    if (len < 0 || len > cap) {
        return -1;
    }
    llama_token bos = llama_vocab_bos(h->vocab);
    if (bos != LLAMA_TOKEN_NULL) {
        const char *b = llama_vocab_get_text(h->vocab, bos);
        size_t k = b ? strlen(b) : 0;
        if (k > 0 && (size_t)len >= k && memcmp(out, b, k) == 0) {
            memmove(out, out + k, (size_t)len - k);
            len -= (int)k;
        }
    }
    return len;
}

// Counted as a prompt is, so chat marks are one token each; the BOS is not.
int nai_count_tokens(const nai *h, const char *text, int len) {
    int n = llama_tokenize(h->vocab, text, len, NULL, 0, false, true);
    return n < 0 ? -n : n;
}

void nai_last(const nai *h, nai_stats *s) {
    s->n_prompt = h->n_prompt;
    s->n_cached = h->n_cached;
    s->n_out = h->n_out;
    s->n_kept = h->n_kept;
    s->n_left = h->n_left;
    s->prefill_ms = (double)h->prefill_us / 1000.0;
    s->decode_ms = (double)h->decode_us / 1000.0;
}

// The next run copies `text` (the answer is expected to be it, nearly) and
// leaves it only where the model prefers another token by a log-odds margin:
// `near` for a token spelled like the one written (a typo, an agreement, a
// kana), `far` for anything else (a synonym, a dropped word, a translation).
// A small model otherwise "fixes" what was right.
int nai_anchor(nai *h, const char *text, int len, float near, float far) {
    free(h->src);
    h->src = NULL;
    h->n_src = 0;
    h->near = near;
    h->far = far;
    int n = llama_tokenize(h->vocab, text, len, NULL, 0, false, false);
    n = n < 0 ? -n : n;
    if (n == 0) {
        return 0;
    }
    h->src = malloc(sizeof *h->src * (size_t)n);
    if (!h->src || llama_tokenize(h->vocab, text, len, h->src, n, false, false) != n) {
        free(h->src);
        h->src = NULL;
        return NAI_FAILED;
    }
    h->n_src = n;
    return n;
}

// The first `len` bytes of the next prompt are an opening many prompts
// share: its state is kept after the first time and restored after that.
void nai_prefix(nai *h, int len) {
    h->keep = len;
}

static nai_opening *find_prefix(nai *h, const char *prompt, int len) {
    for (int i = 0; i < NAI_PREFIXES; i++) {
        nai_opening *p = &h->kept[i];
        if (p->text && p->len == len && memcmp(p->text, prompt, (size_t)len) == 0) {
            return p;
        }
    }
    return NULL;
}

static void keep_prefix(nai *h, const char *prompt, int len, int n_tokens) {
    size_t size = llama_state_seq_get_size(h->ctx, 0);
    uint8_t *state = malloc(size);
    char *text = malloc((size_t)len);
    if (!state || !text || llama_state_seq_get_data(h->ctx, state, size, 0) != size) {
        free(state);
        free(text);
        return;
    }
    memcpy(text, prompt, (size_t)len);
    nai_opening *p = &h->kept[h->next_kept];
    h->next_kept = (h->next_kept + 1) % NAI_PREFIXES;
    free(p->text);
    free(p->state);
    *p = (nai_opening){text, len, n_tokens, state, size};
}

static int tokenize(const nai *h, const char *text, int len, bool bos, llama_token **out) {
    *out = NULL;
    int n = llama_tokenize(h->vocab, text, len, NULL, 0, bos, true);
    n = n < 0 ? -n : n;
    if (n == 0) {
        return 0;
    }
    *out = malloc(sizeof **out * (size_t)n);
    if (!*out || llama_tokenize(h->vocab, text, len, *out, n, bos, true) != n) {
        free(*out);
        *out = NULL;
        return -1;
    }
    return n;
}

static int prefill(nai *h, llama_token *toks, int n, const volatile int *cancel) {
    for (int i = 0; i < n; i += h->n_batch) {
        int m = n - i < h->n_batch ? n - i : h->n_batch;
        int rc = llama_decode(h->ctx, llama_batch_get_one(toks + i, m));
        if (rc == 2 || (cancel && *cancel)) {
            return NAI_CANCELLED;
        }
        if (rc != 0) {
            return NAI_FAILED;
        }
    }
    return NAI_OK;
}

static llama_token argmax(const float *logits, int n) {
    llama_token best = 0;
    for (int i = 1; i < n; i++) {
        if (logits[i] > logits[best]) {
            best = i;
        }
    }
    return best;
}

static bool allowed(struct llama_sampler *grammar, llama_token t, float logit) {
    if (!grammar) {
        return true;
    }
    llama_token_data one = {t, logit, 0.0f};
    llama_token_data_array cur = {&one, 1, -1, false};
    llama_sampler_apply(grammar, &cur);
    return isfinite(one.logit);
}

// The greedy token the grammar allows. The unconstrained best is tried
// first, since it is nearly always allowed; only when it is not is the
// whole vocabulary filtered.
static llama_token pick(nai *h, struct llama_sampler *grammar, const float *logits) {
    llama_token best = argmax(logits, h->n_vocab);
    if (allowed(grammar, best, logits[best])) {
        return best;
    }
    for (int i = 0; i < h->n_vocab; i++) {
        h->all[i] = (llama_token_data){i, logits[i], 0.0f};
    }
    llama_token_data_array full = {h->all, (size_t)h->n_vocab, -1, false};
    llama_sampler_apply(grammar, &full);
    llama_token found = LLAMA_TOKEN_NULL;
    float top = -INFINITY;
    for (size_t i = 0; i < full.size; i++) {
        if (full.data[i].logit > top) {
            top = full.data[i].logit;
            found = full.data[i].id;
        }
    }
    return found;
}

static int piece_of(const nai *h, llama_token t, char *buf, int cap) {
    int k = llama_token_to_piece(h->vocab, t, buf, cap, 0, false);
    return k > 0 ? k : 0;
}

// Code points of UTF-8 `s`, up to `cap`, stopping before a character cut
// off; Arabic vowel marks are left out, since adding them is not a fix.
static int decode(const char *s, int k, uint32_t *cp, int cap) {
    int i = 0, n = 0;
    while (i < k && n < cap) {
        unsigned char c = (unsigned char)s[i];
        int m = c < 0x80 ? 1 : (c >> 5) == 6 ? 2 : (c >> 4) == 14 ? 3 : (c >> 3) == 30 ? 4 : 0;
        if (m == 0 || i + m > k) {
            break;
        }
        uint32_t u = m == 1 ? c : m == 2 ? c & 0x1Fu : m == 3 ? c & 0x0Fu : c & 0x07u;
        for (int j = 1; j < m; j++) {
            u = (u << 6) | ((unsigned char)s[i + j] & 0x3Fu);
        }
        if (!((u >= 0x064B && u <= 0x0652) || u == 0x0670)) {
            cp[n++] = u;
        }
        i += m;
    }
    return n;
}

static bool is_letter(uint32_t u) {
    if (u < 0x80) {
        return (u | 32) >= 'a' && (u | 32) <= 'z';
    }
    if (u < 0xC0 || u == 0xD7 || u == 0xF7) {
        return false;
    }
    return !(u >= 0x2000 && u < 0x2070) && !(u >= 0x3000 && u < 0x3040) && !(u >= 0xFF00 && u < 0xFF66) && !(u >= 0x0660 && u <= 0x066C) && u != 0x060C &&
           u != 0x061B && u != 0x061F;
}

// A letter, or an apostrophe within a word (can't, l'année): putting one
// in is a respelling, not punctuation.
static bool wordish(uint32_t u) {
    return is_letter(u) || u == 0x27 || u == 0x2019;
}

// A token that is only spaces (the second of a double space).
static bool blank(const nai *h, llama_token t) {
    char p[16];
    int k = piece_of(h, t, p, (int)sizeof p);
    for (int i = 0; i < k; i++) {
        if (p[i] != ' ') {
            return false;
        }
    }
    return k > 0;
}

static uint32_t fold(uint32_t u) {
    return (u >= 'A' && u <= 'Z') || (u >= 0xC0 && u <= 0xDE && u != 0xD7) ? u + 32 : u;
}

static int distance(const uint32_t *a, int na, const uint32_t *b, int nb) {
    int row[49];
    for (int j = 0; j <= nb; j++) {
        row[j] = j;
    }
    for (int i = 1; i <= na; i++) {
        int diag = row[0];
        row[0] = i;
        for (int j = 1; j <= nb; j++) {
            int up = row[j];
            int best = diag + (fold(a[i - 1]) != fold(b[j - 1]));
            best = up + 1 < best ? up + 1 : best;
            best = row[j - 1] + 1 < best ? row[j - 1] + 1 : best;
            row[j] = best;
            diag = up;
        }
    }
    return row[nb];
}

static int script(uint32_t u) {
    if (u < 0x250) {
        return 1;
    }
    if (u >= 0x370 && u < 0x530) {
        return 2;
    }
    if (u >= 0x590 && u < 0x900) {
        return 3;
    }
    if ((u >= 0x1100 && u < 0x1200) || (u >= 0x3130 && u < 0x3190) || (u >= 0xAC00 && u < 0xD7B0)) {
        return 4;
    }
    if (u >= 0x3000 && u < 0xA000) {
        return 5;
    }
    return 0;
}

static uint32_t hiragana(uint32_t u) {
    return u >= 0x30A1 && u <= 0x30F6 ? u - 0x60 : u;
}

static bool is_kana(uint32_t u) {
    return u >= 0x3041 && u <= 0x30FA;
}

// Kana people write for one another: voiced or not, small or not, and the
// particles spelled as they sound (わ for は, え for へ, お for を). Any other
// kana for a kana changes what is said (a tense, a particle).
static bool kana_alike(uint32_t a, uint32_t b) {
    static const uint32_t groups[][4] = {
        {0x304B, 0x304C}, {0x304D, 0x304E}, {0x304F, 0x3050}, {0x3051, 0x3052}, {0x3053, 0x3054}, {0x3055, 0x3056}, {0x3057, 0x3058},
        {0x3059, 0x305A}, {0x305B, 0x305C}, {0x305D, 0x305E}, {0x305F, 0x3060}, {0x3061, 0x3062}, {0x3064, 0x3065, 0x3063}, {0x3066, 0x3067},
        {0x3068, 0x3069}, {0x306F, 0x3070, 0x3071, 0x308F}, {0x3072, 0x3073, 0x3074}, {0x3075, 0x3076, 0x3077}, {0x3078, 0x3079, 0x307A, 0x3048},
        {0x307B, 0x307C, 0x307D}, {0x3042, 0x3041}, {0x3044, 0x3043}, {0x3046, 0x3045}, {0x3048, 0x3047}, {0x304A, 0x3049, 0x3092},
        {0x3084, 0x3083}, {0x3086, 0x3085}, {0x3088, 0x3087},
    };
    a = hiragana(a);
    b = hiragana(b);
    for (size_t g = 0; g < sizeof groups / sizeof *groups; g++) {
        bool ia = false, ib = false;
        for (int i = 0; i < 4 && groups[g][i]; i++) {
            ia |= groups[g][i] == a;
            ib |= groups[g][i] == b;
        }
        if (ia && ib) {
            return true;
        }
    }
    return false;
}

// Arabic letters people write for one another: the forms of alif and
// hamza, ta marbuta and ha, alif maqsura and ya.
static bool arabic_alike(uint32_t a, uint32_t b) {
    static const uint32_t groups[][5] = {{0x0627, 0x0623, 0x0625, 0x0622, 0x0621}, {0x0624, 0x0626, 0x0621, 0x0623, 0}, {0x0647, 0x0629, 0x062A, 0, 0}, {0x0649, 0x064A, 0, 0, 0}};
    for (size_t g = 0; g < sizeof groups / sizeof *groups; g++) {
        bool ia = false, ib = false;
        for (int i = 0; i < 5 && groups[g][i]; i++) {
            ia |= groups[g][i] == a;
            ib |= groups[g][i] == b;
        }
        if (ia && ib) {
            return true;
        }
    }
    return false;
}

// Hangul syllables that start with the same consonant (plain or tense): a
// Korean misspelling changes the vowel or the final, not how it opens.
static bool hangul_alike(uint32_t a, uint32_t b) {
    // ㄲ ㄸ ㅃ ㅆ ㅉ fold to ㄱ ㄷ ㅂ ㅅ ㅈ.
    static const int plain[19] = {0, 0, 2, 3, 3, 5, 6, 7, 7, 9, 9, 11, 12, 12, 14, 15, 16, 17, 18};
    return plain[(a - 0xAC00) / 588] == plain[(b - 0xAC00) / 588];
}

enum { SAME, NEAR, FAR, NEVER };

// How the model's token `t` compares with what is written from token `at`
// on: the same text split another way; a respelling of it (a slip of a
// letter or two, an ending, a kana, the case of a letter); another word, a
// word dropped or added, a translation; or punctuation or a number
// changed, which a fix never does.
static int compare(const nai *h, llama_token t, const llama_token *src, int at, int n_src, bool first) {
    char tp[64], sp[192];
    int off[5] = {0};
    int kt = piece_of(h, t, tp, (int)sizeof tp);
    int ks = 0;
    for (int j = at; j < n_src && ks < 128; j++) {
        if (j - at < 5) {
            off[j - at] = ks;
        }
        ks += piece_of(h, src[j], sp + ks, (int)sizeof sp - ks);
    }
    if (kt == 0 || ks == 0) {
        return FAR;
    }
    if (kt <= ks && memcmp(tp, sp, (size_t)kt) == 0) {
        return SAME;
    }
    uint32_t a[32], b[48];
    int ia = tp[0] == ' ';
    int na = decode(tp + ia, kt - ia, a, 32);
    if (na == 0) {
        return FAR;
    }
    for (int i = 0; i < na; i++) {
        if (!wordish(a[i])) {
            return NEVER;
        }
    }
    // Writing now what comes a few tokens on drops what is between.
    for (int s = 1; s <= 3 && at + s < n_src; s++) {
        int o = off[s] + (sp[off[s]] == ' ');
        int n = decode(sp + o, ks - o, b, na);
        bool same = n == na;
        for (int i = 0; same && i < na; i++) {
            same = fold(a[i]) == fold(b[i]);
        }
        if (same) {
            return FAR;
        }
    }
    int ib = sp[0] == ' ';
    int nb = decode(sp + ib, ks - ib, b, 48);
    if (nb == 0) {
        return FAR;
    }
    if (!wordish(b[0])) {
        return NEVER;
    }
    if (script(a[0]) != script(b[0])) {
        return FAR;
    }
    // In words set apart by spaces, one letter is no respelling of a longer
    // word; in Chinese and Japanese a character is a word of its own.
    int word = 0;
    while (word < nb && is_letter(b[word])) {
        word++;
    }
    if (na == 1 && word > 2 && script(a[0]) != 5 && fold(a[0]) != fold(b[0])) {
        return FAR;
    }
    int best = 99, at_len = 0;
    for (int len = na > 3 ? na - 2 : 1; len <= nb && len <= na + 2; len++) {
        int d = distance(a, na, b, len);
        if (d < best) {
            best = d;
            at_len = len;
        }
    }
    if (best == 0) {
        // The same letters: a capital put right, or only marks or spacing.
        return at_len == na && memcmp(a, b, sizeof *a * (size_t)na) != 0 ? NEAR : FAR;
    }
    // Korean: syllables respelled one for one; one put in or taken out is a
    // change of tone (시, 세, 습니다).
    if (script(a[0]) == 4 && at_len != na) {
        return FAR;
    }
    if (at_len == na) {
        // Letters put one for another, only as each script's slips go:
        // a kana for its sound-alike, a Hangul syllable opening as written,
        // an Arabic letter for its look-alike.
        for (int i = 0; i < na; i++) {
            uint32_t x = a[i], y = b[i];
            if (fold(x) == fold(y)) {
                continue;
            }
            bool hangul = x >= 0xAC00 && x <= 0xD7A3 && y >= 0xAC00 && y <= 0xD7A3;
            bool arabic = x >= 0x0600 && x < 0x0700 && y >= 0x0600 && y < 0x0700;
            bool kana = is_kana(x) && is_kana(y);
            if ((kana && !kana_alike(x, y)) || (hangul && !hangul_alike(x, y)) || (arabic && !arabic_alike(x, y)) || (script(a[0]) == 4 && !hangul)) {
                return FAR;
            }
        }
    }
    // The first character of Chinese, Japanese or Korean is never put for
    // another: that is a new sentence.
    if (first && na == 1 && script(a[0]) >= 4 && a[0] != b[0]) {
        return FAR;
    }
    return best <= (na / 3 > 1 ? na / 3 : 1) ? NEAR : FAR;
}

// One answer to `prompt` (ChatML, without the BOS the vocab adds), greedy,
// held to `grammar` (GBNF) when given. Stops at the end of the turn, at
// `max_tokens` (NAI_LONG), or when *cancel turns nonzero (NAI_CANCELLED).
int nai_run(nai *h, const char *prompt, int prompt_len, const char *grammar, int max_tokens, const volatile int *cancel, char *out, int cap,
            int *out_len) {
    *out_len = 0;
    h->cancel = cancel;
    h->n_prompt = 0;
    h->n_out = 0;
    h->n_kept = 0;
    h->n_left = 0;
    h->prefill_us = 0;
    h->decode_us = 0;
    // An anchor holds for one run.
    llama_token *src = h->src;
    int n_src = h->n_src;
    h->src = NULL;
    h->n_src = 0;
    int keep = h->keep > 0 && h->keep < prompt_len ? h->keep : 0;
    h->keep = 0;
    h->n_cached = 0;
    llama_token *head = NULL;
    llama_token *rest = NULL;
    struct llama_sampler *g = NULL;
    int status = NAI_OK;
    int len = 0;
    llama_memory_t mem = llama_get_memory(h->ctx);
    llama_memory_clear(mem, true);

    // The opening is read once and restored after; what follows it is
    // tokenized apart, which splits the same way since the opening ends on
    // a chat mark.
    int64_t t0 = llama_time_us();
    nai_opening *hit = keep ? find_prefix(h, prompt, keep) : NULL;
    if (hit && llama_state_seq_set_data(h->ctx, hit->state, hit->size, 0) != hit->size) {
        llama_memory_clear(mem, true);
        hit = NULL;
    }
    int n_head = hit ? hit->n_tokens : tokenize(h, prompt, keep ? keep : prompt_len, true, &head);
    int n_rest = keep ? tokenize(h, prompt + keep, prompt_len - keep, false, &rest) : 0;
    if (n_head < 0 || n_rest < 0) {
        status = NAI_FAILED;
        goto done;
    }
    int n = n_head + n_rest;
    if (n == 0 || n + max_tokens > nai_n_ctx(h)) {
        status = NAI_TOO_BIG;
        goto done;
    }
    h->n_prompt = n;
    h->n_cached = hit ? n_head : 0;
    if (grammar) {
        g = llama_sampler_init_grammar(h->vocab, grammar, "root");
        if (!g) {
            status = NAI_BAD_GRAMMAR;
            goto done;
        }
    }
    if (!hit) {
        status = prefill(h, head, n_head, cancel);
        if (status == NAI_OK && keep) {
            keep_prefix(h, prompt, keep, n_head);
        }
    }
    if (status == NAI_OK) {
        status = prefill(h, rest, n_rest, cancel);
    }
    int64_t t1 = llama_time_us();
    h->prefill_us = t1 - t0;

    // With an anchor: `at` is the next token of it to copy while the answer
    // follows it; once the answer leaves it, `left` is where, and the first
    // token of the anchor it writes again, a few on, brings it back.
    bool anchored = n_src > 0;
    bool following = anchored;
    int at = 0;
    int left = 0;
    // The tokens of the first word.
    int opening = 1;
    while (opening < n_src) {
        char p[64];
        int k = piece_of(h, src[opening], p, (int)sizeof p);
        uint32_t u[1];
        if (k == 0 || p[0] == ' ' || decode(p, k, u, 1) == 0 || !is_letter(u[0]) || script(u[0]) == 5) {
            break;
        }
        opening++;
    }
    while (status == NAI_OK) {
        if (h->n_out >= max_tokens) {
            status = NAI_LONG;
            break;
        }
        const float *logits = llama_get_logits_ith(h->ctx, -1);
        if (nai_tracing && nai_log_file && h->n_out == 0) {
            llama_token top[3] = {0, 0, 0};
            for (int i = 0; i < h->n_vocab; i++) {
                for (int j = 0; j < 3; j++) {
                    if (logits[i] > logits[top[j]] || (i == 0 && j == 0)) {
                        memmove(top + j + 1, top + j, sizeof *top * (size_t)(2 - j));
                        top[j] = i;
                        break;
                    }
                }
            }
            for (int j = 0; j < 3; j++) {
                char p[64];
                int k = piece_of(h, top[j], p, (int)sizeof p - 1);
                p[k] = 0;
                fprintf(nai_log_file, "  first choice %d: '%s' %.2f\n", j + 1, p, logits[top[j]]);
            }
        }
        llama_token t = pick(h, g, logits);
        if (t == LLAMA_TOKEN_NULL) {
            status = NAI_FAILED;
            break;
        }
        if (following) {
            llama_token copy = at < n_src ? src[at] : h->end;
            if (t != copy) {
                // Ending early or going on past the end is never a fix. The
                // first word is changed more surely than others: another
                // change there is most often the model answering, rewording
                // or translating. A space too many is dropped as a slip.
                bool first = at < opening;
                bool over = at >= n_src || llama_vocab_is_eog(h->vocab, t);
                bool spaced = !over && at + 1 < n_src && t == src[at + 1] && blank(h, copy);
                int kind = over ? FAR : spaced ? NEAR : compare(h, t, src, at, n_src, first);
                float gap = logits[t] - logits[copy];
                float margin = over || kind == NEVER || kind == SAME ? INFINITY : kind == NEAR ? (first ? h->near + 3.0f : h->near) : first ? h->far + 3.0f : h->far;
                bool keep = gap < margin && allowed(g, copy, logits[copy]);
                if (nai_tracing && nai_log_file) {
                    char a[64], b[64];
                    int ka = piece_of(h, t, a, (int)sizeof a - 1);
                    int kb = piece_of(h, copy, b, (int)sizeof b - 1);
                    a[ka] = 0;
                    b[kb] = 0;
                    const char *what = kind == SAME ? "same" : kind == NEAR ? "near" : kind == FAR ? "far" : "never";
                    fprintf(nai_log_file, "  at %d: wants '%s' over '%s' by %.2f, %s, %s\n", at, a, b, gap, what, keep ? "kept" : "changed");
                }
                if (keep) {
                    t = copy;
                    h->n_kept++;
                }
            }
        }
        if (llama_vocab_is_eog(h->vocab, t)) {
            break;
        }
        if (g) {
            llama_sampler_accept(g, t);
        }
        if (following) {
            if (at < n_src && t == src[at]) {
                at++;
            } else {
                following = false;
                left = at;
                h->n_left++;
            }
        } else if (anchored) {
            for (int j = left; j < n_src && j < left + 8; j++) {
                if (src[j] == t) {
                    at = j + 1;
                    following = true;
                    break;
                }
            }
        }
        char piece[256];
        int k = llama_token_to_piece(h->vocab, t, piece, (int)sizeof piece, 0, false);
        if (k < 0 || len + k > cap) {
            status = NAI_FAILED;
            break;
        }
        memcpy(out + len, piece, (size_t)k);
        len += k;
        h->n_out++;
        int rc = llama_decode(h->ctx, llama_batch_get_one(&t, 1));
        if (rc == 2 || (cancel && *cancel)) {
            status = NAI_CANCELLED;
        } else if (rc != 0) {
            status = NAI_FAILED;
        }
    }
    h->decode_us = llama_time_us() - t1;
done:
    if (g) {
        llama_sampler_free(g);
    }
    free(head);
    free(rest);
    free(src);
    *out_len = len;
    h->cancel = NULL;
    return status;
}
