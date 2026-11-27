//! Background text recognition for image clips, so an image can be found by the words in it
//! (`ditto_platform::ocr`; Ditto has no equivalent).
//!
//! One worker thread reads the waiting images one at a time, newest first: a new copy is
//! searchable within a second or so, and an existing history is caught up on in the
//! background. Recognition never runs while the history is locked.

use std::ops::DerefMut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ditto_core::store::Store;
use ditto_platform::ocr;
use tauri::AppHandle;

use crate::state::{emit_clips_changed, state};

/// Let the app finish starting before the first image is read.
const START_DELAY: Duration = Duration::from_secs(3);
/// Breathing space between images while catching up on a history.
const PAUSE: Duration = Duration::from_millis(100);
/// While catching up, refresh an open quick-paste window at most this often.
const REFRESH_EVERY: Duration = Duration::from_secs(2);
/// Stop for now after this many failures in a row: the engine itself is probably the problem.
const MAX_FAILURES_IN_A_ROW: usize = 3;

pub fn start(app: &AppHandle) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<()>();
    *state(app).lock_image_text_wake() = Some(tx);
    let handle = app.clone();
    std::thread::Builder::new().name("image-text".into()).spawn(move || {
        std::thread::sleep(START_DELAY);
        // Images that could not be read; skipped until the next start.
        let mut failed = Vec::new();
        loop {
            read_waiting(&handle, &mut failed);
            if rx.recv().is_err() {
                return;
            }
            while rx.try_recv().is_ok() {}
        }
    })?;
    Ok(())
}

/// There may be images to read: something was copied or imported, or the option was switched on.
pub fn wake(app: &AppHandle) {
    if let Some(tx) = state(app).lock_image_text_wake().as_ref() {
        let _ = tx.send(());
    }
}

fn read_waiting(app: &AppHandle, failed: &mut Vec<i64>) {
    static UNAVAILABLE_LOGGED: AtomicBool = AtomicBool::new(false);
    let st = state(app);
    let mut unseen_results = false;
    let mut last_refresh = Instant::now();
    let mut failures_in_a_row = 0;
    loop {
        if !st.settings().read_image_text {
            break;
        }
        let mut engine_missing = None;
        let step = read_next(|| st.store(), failed, |png| {
            let status = ocr::status();
            if !status.available {
                engine_missing = Some(status);
                return Err(String::new());
            }
            ocr::recognize_png(png).map_err(|e| e.to_string())
        });
        if let Some(status) = engine_missing {
            // Nothing was wrong with the image: forget the failure so it is read once an
            // engine is installed.
            failed.pop();
            if !UNAVAILABLE_LOGGED.swap(true, Ordering::Relaxed) {
                log::info!("not reading the text in images: {} is unavailable. {}", status.engine, status.detail);
            }
            break;
        }
        match step {
            Step::Idle => break,
            Step::Read { id, characters } => {
                log::debug!("read {characters} character(s) of text in image clip {id}");
                failures_in_a_row = 0;
                unseen_results |= characters > 0;
            }
            Step::Failed { id, error } => {
                log::warn!("could not read the text in image clip {id}: {error}");
                failures_in_a_row += 1;
                if failures_in_a_row >= MAX_FAILURES_IN_A_ROW {
                    break;
                }
            }
        }
        // A search that is on screen may now match this image.
        if unseen_results && last_refresh.elapsed() >= REFRESH_EVERY {
            emit_clips_changed(app);
            unseen_results = false;
            last_refresh = Instant::now();
        }
        std::thread::sleep(PAUSE);
    }
    if unseen_results {
        emit_clips_changed(app);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// No image is waiting.
    Idle,
    Read { id: i64, characters: usize },
    Failed { id: i64, error: String },
}

/// Read the newest waiting image and save its text. `lock` is only held to fetch the image
/// and to save the result, never while `recognize` runs. An image that fails is added to
/// `failed` and not tried again.
fn read_next<G: DerefMut<Target = Store>>(
    lock: impl Fn() -> G,
    failed: &mut Vec<i64>,
    recognize: impl FnOnce(&[u8]) -> Result<String, String>,
) -> Step {
    let next = lock().next_image_to_read(failed);
    let job = match next {
        Ok(Some(job)) => job,
        Ok(None) => return Step::Idle,
        Err(e) => {
            log::warn!("could not look for images to read: {e}");
            return Step::Idle;
        }
    };
    let saved = recognize(&job.png).and_then(|text| {
        let characters = text.trim().chars().count();
        // `false` means the clip changed or was deleted meanwhile, which is fine.
        lock().set_image_text(job.id, job.hash, &text).map(|_| characters).map_err(|e| e.to_string())
    });
    match saved {
        Ok(characters) => Step::Read { id: job.id, characters },
        Err(error) => {
            failed.push(job.id);
            Step::Failed { id: job.id, error }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ditto_core::ClipContent;
    use ditto_core::store::AddOptions;

    use super::*;

    /// A small PNG; different sizes give different images.
    fn png(side: u32) -> Vec<u8> {
        let sample = include_bytes!("../../crates/ditto-platform/tests/fixtures/ocr-sample.png");
        ditto_core::imaging::thumbnail_png(sample, side, side).unwrap()
    }

    #[test]
    fn reads_the_newest_image_first_without_holding_the_lock() {
        let store = Mutex::new(Store::open_in_memory().unwrap());
        let add = |content: ClipContent| store.lock().unwrap().add_clip(&content, &AddOptions::default()).unwrap().id();
        let older = add(ClipContent::from_png(png(40)));
        add(ClipContent::from_text("not an image"));
        let newer = add(ClipContent::from_png(png(50)));
        let lock = || store.lock().unwrap();
        let mut failed = Vec::new();

        let step = read_next(lock, &mut failed, |bytes| {
            assert_eq!(bytes, png(50));
            assert!(store.try_lock().is_ok(), "the history must not be locked during recognition");
            Ok("  Hello\nworld ".into())
        });
        assert_eq!(step, Step::Read { id: newer, characters: 11 });
        assert_eq!(store.lock().unwrap().image_text(newer).unwrap().as_deref(), Some("Hello\nworld"));

        // A failure is remembered and skipped; the image stays unread for the next start.
        let step = read_next(lock, &mut failed, |_| Err("engine exploded".into()));
        assert_eq!(step, Step::Failed { id: older, error: "engine exploded".into() });
        assert_eq!(failed, vec![older]);
        assert_eq!(read_next(lock, &mut failed, |_| panic!("nothing left to read")), Step::Idle);
        assert_eq!(store.lock().unwrap().image_text(older).unwrap(), None);

        // An image with no text is recorded as read, so it is not read again.
        failed.clear();
        assert_eq!(read_next(lock, &mut failed, |_| Ok(String::new())), Step::Read { id: older, characters: 0 });
        assert_eq!(read_next(lock, &mut failed, |_| panic!("nothing left to read")), Step::Idle);
    }

    #[test]
    fn a_result_for_a_deleted_clip_is_dropped_quietly() {
        let store = Mutex::new(Store::open_in_memory().unwrap());
        let id = store.lock().unwrap().add_clip(&ClipContent::from_png(png(40)), &AddOptions::default()).unwrap().id();
        let mut failed = Vec::new();
        let step = read_next(|| store.lock().unwrap(), &mut failed, |_| {
            store.lock().unwrap().delete_clips(&[id]).unwrap();
            Ok("text".into())
        });
        assert_eq!(step, Step::Read { id, characters: 4 });
        assert!(failed.is_empty());
    }
}
