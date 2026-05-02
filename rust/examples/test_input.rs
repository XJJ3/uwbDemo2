use std::time::Duration;
use std::thread;

fn main() {
    crossterm::terminal::enable_raw_mode().ok();
    
    let (tx, rx) = crossbeam_channel::bounded(64);
    
    thread::spawn(move || {
        loop {
            if crossterm::event::poll(Duration::from_millis(100)).is_err() {
                continue;
            }
            
            if let Ok(crossterm::event::Event::Key(key)) = crossterm::event::read() {
                if let crossterm::event::KeyCode::Char(c) = key.code {
                    let _ = tx.send(c.to_string());
                }
            }
        }
    });
    
    loop {
        match rx.try_recv() {
            Ok(c) => eprintln!("Got: {}", c),
            Err(_) => {}
        }
        thread::sleep(Duration::from_millis(1));
    }
}
