//! The first run's writing model card (kolada-ui `modelcard`): Light or
//! Full, or Not now; once chosen it springs down to a strip with the
//! download's progress, and goes when the model is in place.

use super::*;
use super::assist::Fetch;
use kolada_assist::models::{self, MODELS};
use kolada_ui::modelcard::{Choice, Command, Hit, ModelCard, ModelCardView, Phase};

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

/// A spring with a little overshoot, 0 to 1 over `t` in 0..1.
fn spring(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t >= 1.0 {
        return 1.0;
    }
    1.0 - (-6.5 * t).exp() * (9.0 * t).cos()
}

impl App {
    /// After the first frame: a model already here is taken as the choice
    /// without asking; else, the first time, the card is offered where a
    /// model can be had (the helper is here and the catalogue says where).
    /// A model chosen before and not here yet carries on downloading.
    pub(super) fn offer_model(&mut self) {
        if self.compact {
            return;
        }
        if self.session.model.is_empty() {
            if let Some(dir) = crate::assist::models_dir() {
                if let Some(m) = MODELS.iter().rev().find(|m| kolada_core::fetch::present(&dir, m.file, m.bytes)) {
                    self.session.model = m.id.to_string();
                    self.session.model_asked = true;
                    return;
                }
            }
        }
        if !self.session.model_asked {
            let offered = crate::assist::helper_here() && MODELS.first().is_some_and(|m| kolada_core::fetch::address(models::url(m), m.file).is_some());
            if offered {
                self.model_card = Some(Shown { card: ModelCard::new(choices()), opened: Instant::now(), strip: (0.0, 0.0, Instant::now()), closing: None, picked_at: Instant::now() - GLIDE });
                self.request_frame();
            }
            return;
        }
        if self.session.assist == 0 && models::by_id(&self.session.model).is_some() {
            self.download_model();
        }
    }

    pub(super) fn model_view(&self) -> Option<ModelCardView<'_>> {
        let s = self.model_card.as_ref()?;
        let shown = chrome::ease(s.opened.elapsed().as_secs_f32() / FADE.as_secs_f32());
        let gone = s.closing.map_or(1.0, |at| 1.0 - chrome::ease(at.elapsed().as_secs_f32() / FADE.as_secs_f32()));
        let (from, to, at) = s.strip;
        // A spring set to start later (Download fills first) waits.
        let springing = Instant::now().saturating_duration_since(at).as_secs_f32();
        let strip = from + (to - from) * spring(springing / SPRING.as_secs_f32());
        let glide = spring(s.picked_at.elapsed().as_secs_f32() / GLIDE.as_secs_f32());
        Some(ModelCardView { card: &s.card, alpha: shown.min(gone), strip, since: s.opened.elapsed().as_secs_f32(), glide })
    }

    /// The card is still moving (fading or springing): frames keep coming.
    pub(super) fn model_card_moving(&self) -> bool {
        self.model_card.as_ref().is_some_and(|s| {
            s.opened.elapsed() < ARRIVE.max(FADE) + Duration::from_millis(20)
                || Instant::now() < s.strip.2 + SPRING + Duration::from_millis(20)
                || s.picked_at.elapsed() < GLIDE + Duration::from_millis(20)
                || s.closing.is_some()
        })
    }

    /// Called each frame: a card that has faded away is dropped.
    pub(super) fn model_card_tick(&mut self) {
        if self.model_card.as_ref().is_some_and(|s| s.closing.is_some_and(|at| at.elapsed() > FADE)) {
            self.model_card = None;
        }
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
        self.request_frame();
    }

    pub(super) fn model_click(&mut self, hit: Hit) {
        let Some(s) = &mut self.model_card else { return };
        if s.closing.is_some() {
            return;
        }
        let before = s.card.picked();
        let command = s.card.click(hit);
        if s.card.picked() != before {
            s.picked_at = Instant::now();
        }
        match command {
            Command::None => {}
            Command::Download(i) => {
                let Some(m) = MODELS.get(i) else { return };
                self.session.assist = 0;
                self.choose_model(m.id);
                // The capsule fills in place a moment, then goes down.
                self.spring_card(1.0, FILL_FIRST);
                self.save_session();
            }
            Command::NotNow => {
                // Writing help stays off; Settings › Writing help can choose later.
                self.session.model_asked = true;
                self.session.assist = 1;
                self.assist_setting();
                self.close_card();
                self.save_session();
            }
            Command::Cancel => {
                self.cancel_model_download();
                self.session.model_asked = false;
                self.spring_card(0.0, Duration::ZERO);
            }
        }
        self.request_frame();
    }

    /// The download said something: the strip follows it; done, the card
    /// goes and a whisper says so; failed, the card goes quietly (it is
    /// tried again at the next start, from where it stopped).
    pub(super) fn model_card_follows(&mut self) {
        let (coming, done, failed) = match self.model_fetch() {
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
            self.whisper("Writing help is ready".to_string());
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
}
