//! The first run's writing model card (kolada-ui `modelcard`), as on
//! Windows (`kolada-win` `app/model_card.rs`): Light or Full, or Not now;
//! once chosen it springs down to a strip with the download's progress,
//! and goes when the model is in place ("Writing help is ready"). Offered
//! only where a model can be had: the helper beside the app, and a
//! catalogue that says where models come from.

use std::time::{Duration, Instant};

use kolada_assist::models::{self, MODELS};
use kolada_ui::modelcard::{Choice, Command, Hit, ModelCard, ModelCardView, Phase};

use super::assist::Fetch;
use super::*;

/// The card's fade in and out.
const FADE: Duration = Duration::from_millis(180);
/// The card springing to a strip and back.
const SPRING: Duration = Duration::from_millis(520);
/// Its parts coming in one after another, and the teal highlight gliding.
const ARRIVE: Duration = Duration::from_millis(1100);
const GLIDE: Duration = Duration::from_millis(380);
/// Download fills in place this long before it springs down to the strip.
const FILL_FIRST: Duration = Duration::from_millis(1400);

pub(super) struct Shown {
    pub card: ModelCard,
    opened: Instant,
    /// Springing to the strip (1) or back to the card (0), since when.
    strip: (f32, f32, Instant),
    /// Going away, since when.
    closing: Option<Instant>,
    /// The last pick, when: the teal highlight glides from the one before.
    picked_at: Instant,
}

/// The names the card gives the catalogue's sizes, smallest first.
fn name(id: &str) -> &'static str {
    match id {
        "light" => "Light",
        "full" => "Full",
        _ => "Model",
    }
}

fn choices() -> Vec<Choice> {
    MODELS.iter().map(|m| Choice { id: m.id, name: name(m.id), bytes: m.bytes }).collect()
}

/// The largest of the catalogue's models whole in `dir`, if any.
fn here(dir: &Path) -> Option<&'static models::Model> {
    MODELS.iter().rev().find(|m| kolada_core::fetch::present(dir, m.file, m.bytes))
}

/// A spring with a little overshoot, 0 to 1 over `t` in 0..1.
fn spring(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t >= 1.0 {
        return 1.0;
    }
    1.0 - (-6.5 * t).exp() * (9.0 * t).cos()
}

impl Model {
    /// After the first frame: a model already here is taken as the choice
    /// without asking; else, the first time, the card is offered where a
    /// model can be had (`offered`: the helper is here and the catalogue
    /// says where). A model chosen before and not here yet carries on
    /// downloading.
    pub(super) fn offer_model(&mut self, offered: bool) {
        if self.compact {
            return;
        }
        if self.session.model.is_empty() {
            if let Some(m) = self.models_dir().and_then(|dir| here(&dir)) {
                self.session.model = m.id.to_string();
                self.session.model_asked = true;
                return;
            }
        }
        if !self.session.model_asked {
            if offered {
                let now = Instant::now();
                self.model_card = Some(Shown { card: ModelCard::new(choices()), opened: now, strip: (0.0, 0.0, now), closing: None, picked_at: now - GLIDE });
            }
            return;
        }
        if self.session.assist == 0 && models::by_id(&self.session.model).is_some() {
            self.download_model();
        }
    }

    /// The card as a first run shows it, for dumps.
    pub fn demo_model_card(&mut self) {
        self.session.model_asked = false;
        self.session.model.clear();
        self.ui.animate = false;
        self.offer_model(true);
    }

    /// Whether this app can offer a model: its helper is beside it and the
    /// catalogue (or a test instance's local server) says where the first
    /// model comes from.
    pub(super) fn model_offered() -> bool {
        crate::helper::helper_here() && MODELS.first().is_some_and(|m| kolada_core::fetch::address(models::url(m), m.file).is_some())
    }

    /// The card as the scene draws it.
    pub(super) fn model_view(&self) -> Option<ModelCardView<'_>> {
        let s = self.model_card.as_ref()?;
        let ease = |t: f32| if self.ui.animate { chrome::ease(t) } else { 1.0 };
        let shown = ease(s.opened.elapsed().as_secs_f32() / FADE.as_secs_f32());
        let gone = s.closing.map_or(1.0, |at| 1.0 - ease(at.elapsed().as_secs_f32() / FADE.as_secs_f32()));
        let (from, to, at) = s.strip;
        // A spring set to start later (Download fills first) waits.
        let springing = Instant::now().saturating_duration_since(at).as_secs_f32();
        let strip = if self.ui.animate { from + (to - from) * spring(springing / SPRING.as_secs_f32()) } else if Instant::now() >= at { to } else { from };
        let glide = if self.ui.animate { spring(s.picked_at.elapsed().as_secs_f32() / GLIDE.as_secs_f32()) } else { 1.0 };
        let since = if self.ui.animate { s.opened.elapsed().as_secs_f32() } else { ARRIVE.as_secs_f32() };
        Some(ModelCardView { card: &s.card, alpha: shown.min(gone), strip, since, glide })
    }

    /// The card is still moving (fading or springing): frames keep coming.
    pub(super) fn model_card_moving(&self) -> bool {
        self.model_card.as_ref().is_some_and(|s| {
            let margin = Duration::from_millis(20);
            s.closing.is_some()
                || (self.ui.animate
                    && (s.opened.elapsed() < ARRIVE.max(FADE) + margin || Instant::now() < s.strip.2 + SPRING + margin || s.picked_at.elapsed() < GLIDE + margin))
        })
    }

    /// When the card next moves by the clock (Download's fill before the spring).
    pub(super) fn model_card_due(&self) -> Option<Instant> {
        self.model_card.as_ref().map(|s| s.strip.2).filter(|t| *t > Instant::now())
    }

    /// A frame: a card that has faded away goes. Whether the screen changes.
    pub(super) fn step_model_card(&mut self) -> bool {
        let gone = self.model_card.as_ref().is_some_and(|s| s.closing.is_some_and(|at| !self.ui.animate || at.elapsed() > FADE));
        if gone {
            self.model_card = None;
        }
        gone
    }

    /// Springs to the strip (1) or back (0), after `wait`.
    fn spring_card(&mut self, to: f32, wait: Duration) {
        let Some(now) = self.model_view().map(|v| v.strip) else { return };
        if let Some(s) = &mut self.model_card {
            s.strip = (now, to, Instant::now() + wait);
        }
    }

    fn close_card(&mut self) {
        if let Some(s) = &mut self.model_card {
            if s.closing.is_none() {
                s.closing = Some(Instant::now());
            }
        }
    }

    /// The card is up (it has the clicks over the page).
    pub fn model_card_shown(&self) -> bool {
        self.model_card.as_ref().is_some_and(|s| s.closing.is_none())
    }

    /// A press while the card is up: its own, and nothing else (the page
    /// behind it is dimmed). None when there is no card.
    pub(super) fn model_card_click(&mut self, hit: Option<Target>) -> Option<bool> {
        let s = self.model_card.as_ref()?;
        // The strip at the bottom leaves the page to be written in.
        let strip = matches!(s.card.phase, Phase::Downloading { .. });
        match hit {
            Some(Target::Model(hit)) => Some(self.model_click(hit)),
            _ if strip || s.closing.is_some() => None,
            _ => Some(false),
        }
    }

    fn model_click(&mut self, hit: Hit) -> bool {
        let Some(s) = &mut self.model_card else { return false };
        if s.closing.is_some() {
            return false;
        }
        let before = s.card.picked();
        let command = s.card.click(hit);
        if s.card.picked() != before {
            s.picked_at = Instant::now();
        }
        match command {
            Command::None => {}
            Command::Download(i) => {
                let Some(m) = MODELS.get(i) else { return true };
                self.session.assist = 0;
                self.choose_model(m.id);
                // The capsule fills in place a moment, then goes down.
                self.spring_card(1.0, FILL_FIRST);
                self.model_card_follows();
            }
            Command::NotNow => {
                // Writing help stays off; Settings › Writing help can choose later.
                self.session.model_asked = true;
                self.session.assist = 1;
                self.assist_setting();
                self.close_card();
            }
            Command::Cancel => {
                self.cancel_model_download();
                self.session.model_asked = false;
                self.spring_card(0.0, Duration::ZERO);
            }
        }
        true
    }

    /// The download said something: the strip follows it; done, the card
    /// goes and a whisper says so; failed, the card goes quietly (it is
    /// tried again at the next start, from where it stopped).
    pub(super) fn model_card_follows(&mut self) {
        let (coming, done, failed) = match &self.writing.fetch {
            Fetch::Coming(d, t) => (Some((*d, *t)), false, false),
            Fetch::Done => (None, true, false),
            Fetch::Failed(_) => (None, false, true),
            Fetch::Idle => (None, false, false),
        };
        let Some(s) = &mut self.model_card else { return };
        if !matches!(s.card.phase, Phase::Downloading { .. }) {
            return;
        }
        if let Some((d, t)) = coming {
            s.card.progress(d, t);
        } else if done {
            self.close_card();
            self.whisper("Writing help is ready");
        } else if failed {
            self.close_card();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_names_the_catalogue_light_then_full() {
        let c = choices();
        assert_eq!(c.iter().map(|c| c.name).collect::<Vec<_>>(), ["Light", "Full"]);
        assert!(c[0].bytes < c[1].bytes);
    }

    #[test]
    fn the_spring_settles_where_it_goes() {
        assert_eq!(spring(0.0), 0.0);
        assert_eq!(spring(1.0), 1.0);
        let most = (0..100).map(|i| spring(i as f32 / 100.0)).fold(0.0f32, f32::max);
        assert!(most > 1.0 && most < 1.15, "a little overshoot, {most}");
    }

    fn open(test: &str) -> (Model, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kolada-app-modelcard-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Plan.md"), "# Plan\n\nWrite it down.\n").unwrap();
        let mut m = Model::new(dir.clone(), false);
        m.open("Plan.md");
        m.frame(1000, 700, 1.0);
        (m, dir)
    }

    fn click(m: &mut Model, hit: Hit) -> bool {
        m.frame(1000, 700, 1.0);
        let r = m.hits.iter().rev().find(|(_, t)| *t == Target::Model(hit)).map(|(r, _)| *r).unwrap_or_else(|| panic!("{hit:?} is drawn"));
        m.pointer_down((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0, 1, false)
    }

    #[test]
    fn the_first_run_offers_light_or_full_and_not_now_keeps_writing_help_off() {
        let (mut m, dir) = open("notnow");
        m.offer_model(false);
        assert!(!m.model_card_shown(), "no helper or nowhere to download from: never offered");
        m.offer_model(true);
        assert!(m.model_card_shown());
        m.frame(1000, 700, 1.0);
        // The page behind it takes no clicks while it asks.
        let text = m.text().to_string();
        let (x, y) = (m.lines[1].x + 4.0, m.lines[1].y + 4.0);
        assert!(m.model_card_click(scene::hit(&m.hits, x, y)).is_some());
        assert!(click(&mut m, Hit::NotNow));
        assert_eq!((m.session.assist, m.session.model_asked, m.session.model.as_str()), (1, true, ""), "off, and asked: never offered again");
        m.frame(1000, 700, 1.0);
        assert!(m.step_model_card() && m.model_card.is_none(), "gone (no motion in tests)");
        m.offer_model(true);
        assert!(!m.model_card_shown());
        assert_eq!(m.text(), text);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn download_keeps_the_choice_and_a_download_that_cannot_start_closes_the_card() {
        let (mut m, dir) = open("download");
        m.offer_model(true);
        // Full picked, then Download.
        assert!(click(&mut m, Hit::Pick(1)));
        assert!(click(&mut m, Hit::Download));
        assert_eq!((m.session.assist, m.session.model_asked, m.session.model.as_str()), (0, true, "full"));
        // A model with no data (tests) downloads nothing: the card goes
        // quietly, and the next start tries again.
        assert_eq!(m.writing.fetch, Fetch::Failed(kolada_core::fetch::Failure::Network));
        assert!(!m.model_card_shown());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_strip_follows_the_download_and_says_when_writing_help_is_ready() {
        let (mut m, dir) = open("strip");
        m.offer_model(true);
        if let Some(s) = &mut m.model_card {
            s.card.click(Hit::Download);
        }
        m.writing.fetch = Fetch::Coming(50, 200);
        m.model_card_follows();
        assert_eq!(m.model_card.as_ref().map(|s| s.card.percent()), Some(25));
        // While it downloads the page is the person's to write in.
        m.frame(1000, 700, 1.0);
        let (x, y) = (m.lines[1].x + 4.0, m.lines[1].y + 4.0);
        assert!(m.model_card_click(scene::hit(&m.hits, x, y)).is_none());
        m.writing.fetch = Fetch::Done;
        m.model_card_follows();
        assert!(!m.model_card_shown());
        assert_eq!(m.ui.whisper.as_ref().map(|w| w.0.as_str()), Some("Writing help is ready"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_model_already_here_is_taken_without_asking() {
        let dir = std::env::temp_dir().join(format!("kolada-app-modelcard-here-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(here(&dir).is_none());
        let (light, full) = (models::by_id("light").unwrap(), models::by_id("full").unwrap());
        std::fs::File::create(dir.join(light.file)).unwrap().set_len(light.bytes).unwrap();
        assert_eq!(here(&dir).map(|m| m.id), Some("light"));
        // Full, but cut short: still Light.
        std::fs::File::create(dir.join(full.file)).unwrap().set_len(full.bytes - 1).unwrap();
        assert_eq!(here(&dir).map(|m| m.id), Some("light"));
        std::fs::File::options().write(true).open(dir.join(full.file)).unwrap().set_len(full.bytes).unwrap();
        assert_eq!(here(&dir).map(|m| m.id), Some("full"), "the larger of two");
        let _ = std::fs::remove_dir_all(dir);
    }
}
