use super::types::{CleanOffer, Outcome, StartOpts};
use super::Session;
use crate::ui::Chrome;
use cleaner_core::config::Config;
use cleaner_core::test_support::TempDir;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::sync::Arc;
use std::time::Duration;

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    })
}

fn release_key(code: KeyCode) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Release,
        state: KeyEventState::empty(),
    })
}

fn wait_for_ready(session: &mut Session) {
    for _ in 0..200 {
        session.tick();
        if !matches!(session.phase, super::types::Phase::Scanning { .. }) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("session did not become ready in time");
}

#[test]
fn session_lifecycle_scanning_to_ready_and_drawing() {
    let temp = TempDir::new("session-lifecycle");
    temp.write("target/output.bin", b"12345");
    temp.write("src/main.rs", b"fn main() {}");

    let config = Arc::new(Config {
        directories: vec!["target".into()],
        files: vec![".pyc".into()],
        days: None,
        force: false,
    });

    let mut session = Session::start(temp.path().to_path_buf(), config, StartOpts::default());
    assert!(!session.is_exited());

    // During scanning:
    let scan_status = session.status_line();
    assert!(scan_status.is_some());

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| session.draw(f, f.area(), Chrome::Full))
        .unwrap();

    wait_for_ready(&mut session);

    // Ready state:
    let offer = session.clean_offer();
    assert!(matches!(offer, CleanOffer::Ready { dirs: 1, .. }));

    terminal
        .draw(|f| session.draw(f, f.area(), Chrome::Full))
        .unwrap();

    // Trigger help status
    session.show_help_status();
    assert!(session.status_line().is_some());

    // Toggles
    session.toggle_sort();
    session.toggle_delete_confirm();
    session.toggle_clean_confirm();
    session.refresh();
}

#[test]
fn session_scan_cancellation() {
    let temp = TempDir::new("session-cancel");
    temp.write("file.txt", b"hello");

    let config = Arc::new(Config::default());
    let mut session = Session::start(temp.path().to_path_buf(), config, StartOpts::default());

    let outcome = session.handle_event(key(KeyCode::Esc));
    assert_eq!(outcome, Outcome::Exit);
    assert!(session.is_exited());
}

#[test]
fn session_key_events_in_ready_state() {
    let temp = TempDir::new("session-keys");
    temp.write("target/artifact", b"123");
    temp.write("test.pyc", b"456");

    let config = Arc::new(Config {
        directories: vec!["target".into()],
        files: vec![".pyc".into()],
        days: None,
        force: false,
    });

    let mut session = Session::start(temp.path().to_path_buf(), config, StartOpts::default());
    wait_for_ready(&mut session);

    // Test non-press event ignored
    assert_eq!(
        session.handle_event(release_key(KeyCode::Char('s'))),
        Outcome::Continue
    );

    // Sort toggle key 's'
    assert_eq!(
        session.handle_event(key(KeyCode::Char('s'))),
        Outcome::Continue
    );

    // Navigation keys
    assert_eq!(session.handle_event(key(KeyCode::Down)), Outcome::Continue);
    assert_eq!(session.handle_event(key(KeyCode::Up)), Outcome::Continue);
    assert_eq!(session.handle_event(key(KeyCode::End)), Outcome::Continue);
    assert_eq!(session.handle_event(key(KeyCode::Home)), Outcome::Continue);

    // Help key '1'
    assert_eq!(
        session.handle_event(key(KeyCode::Char('1'))),
        Outcome::Continue
    );

    // Clean confirm 'c' and cancel 'n'
    assert_eq!(
        session.handle_event(key(KeyCode::Char('c'))),
        Outcome::Continue
    );
    assert_eq!(
        session.handle_event(key(KeyCode::Char('n'))),
        Outcome::Continue
    );

    // Delete confirm 'd' and cancel 'n'
    assert_eq!(
        session.handle_event(key(KeyCode::Char('d'))),
        Outcome::Continue
    );
    assert_eq!(
        session.handle_event(key(KeyCode::Char('n'))),
        Outcome::Continue
    );

    // Number keys and vim keys. '4' is excluded: it opens Deep Clean, which
    // takes over the keyboard, and is covered by its own test below.
    for k in ['2', '8', '9', 'j', 'k', 'g', 'G', '3', '5', '6', '7'] {
        assert_eq!(
            session.handle_event(key(KeyCode::Char(k))),
            Outcome::Continue
        );
    }
    // Cancel any active confirmation
    let _ = session.handle_event(key(KeyCode::Char('n')));

    // Arrow keys & Enter / Backspace
    assert_eq!(session.handle_event(key(KeyCode::Left)), Outcome::Continue);
    assert_eq!(session.handle_event(key(KeyCode::Right)), Outcome::Continue);
    assert_eq!(
        session.handle_event(key(KeyCode::Backspace)),
        Outcome::Continue
    );
    assert_eq!(
        session.handle_event(key(KeyCode::Char('h'))),
        Outcome::Continue
    );

    // Quit key 'q'
    assert_eq!(session.handle_event(key(KeyCode::Char('q'))), Outcome::Exit);
    assert!(session.is_exited());
}

#[test]
fn session_clean_offer_empty_and_run_clean() {
    let temp = TempDir::new("session-offer-empty");
    temp.write("clean_code.rs", b"fn clean() {}");

    let config = Arc::new(Config {
        directories: vec!["target".into()],
        files: vec![".pyc".into()],
        days: None,
        force: false,
    });

    let mut session = Session::start(temp.path().to_path_buf(), config, StartOpts::default());
    wait_for_ready(&mut session);

    let offer = session.clean_offer();
    assert!(matches!(offer, CleanOffer::Empty { .. }));

    // Test run_clean call
    session.run_clean();
}

/// Deep Clean captures the keyboard, so the browser's quit and delete bindings
/// must not fire while it is open.
#[test]
fn deep_clean_opens_on_four_and_returns_on_escape() {
    let temp = TempDir::new("session-deep");
    temp.write("src/main.rs", b"fn main() {}");

    let config = Arc::new(Config {
        directories: vec!["target".into()],
        files: vec![".pyc".into()],
        days: None,
        force: false,
    });

    let mut session = Session::start(temp.path().to_path_buf(), config, StartOpts::default());
    wait_for_ready(&mut session);

    assert_eq!(
        session.handle_event(key(KeyCode::Char('4'))),
        Outcome::Continue
    );
    assert!(session.in_deep(), "'4' should open Deep Clean");

    // 'q' must not quit the app from inside Deep Clean.
    assert_eq!(
        session.handle_event(key(KeyCode::Char('q'))),
        Outcome::Continue
    );
    assert!(
        !session.is_exited(),
        "leaving Deep Clean must not exit the app"
    );
    assert!(!session.in_deep(), "'q' should return to the browser");

    // Back in the browser, 'q' quits as before.
    assert_eq!(session.handle_event(key(KeyCode::Char('q'))), Outcome::Exit);
}

/// Elevated targets are only handed over when the user has actually run
/// something that produced them.
#[test]
fn no_elevated_work_is_offered_by_default() {
    let temp = TempDir::new("session-elevated");
    temp.write("src/main.rs", b"fn main() {}");

    let config = Arc::new(Config {
        directories: vec!["target".into()],
        files: vec![".pyc".into()],
        days: None,
        force: false,
    });

    let mut session = Session::start(temp.path().to_path_buf(), config, StartOpts::default());
    wait_for_ready(&mut session);

    assert!(session.take_elevated().is_empty());
    session.handle_event(key(KeyCode::Char('4')));
    assert!(
        session.take_elevated().is_empty(),
        "opening the view must not queue elevated work"
    );
}
