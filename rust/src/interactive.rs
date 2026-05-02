use crossbeam_channel::Receiver;
use std::io::{self, BufRead};
use std::thread;

pub enum Command {
    Quit,
    Status,
    Help,
    Unknown(String),
}

pub struct InteractiveTerminal {
    #[allow(dead_code)]
    running: bool,
}

impl InteractiveTerminal {
    pub fn new() -> (Self, Receiver<String>) {
        let rx = spawn_input_thread();
        (Self { running: true }, rx)
    }

    pub fn start(&self) {
        println!();
        println!("Commands: /q=quit, /s=status, /h=help");
        println!();
    }
}

pub fn read_stdin_line() -> Option<String> {
    let mut line = String::new();
    io::stdin().read_line(&mut line).ok()?;
    Some(line.trim().to_string())
}

pub fn parse_command(input: &str) -> Command {
    let trimmed = input.trim();
    match trimmed {
        "/q" | "/quit" | "/exit" => Command::Quit,
        "/s" | "/status" => Command::Status,
        "/h" | "/help" => Command::Help,
        _ if trimmed.starts_with('/') => Command::Unknown(trimmed.to_string()),
        _ => Command::Unknown(trimmed.to_string()),
    }
}

pub fn setup_terminal() -> io::Result<()> {
    Ok(())
}

pub fn restore_terminal() -> io::Result<()> {
    Ok(())
}

pub fn spawn_input_thread() -> Receiver<String> {
    let (tx, rx) = crossbeam_channel::bounded(64);
    
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut reader = stdin.lock();
        let mut line = String::new();
        
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim().to_string();
                    if !trimmed.is_empty() {
                        if tx.send(trimmed).is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });
    
    rx
}

pub struct LineBuffer {
    pub buffer: String,
}

impl LineBuffer {
    pub fn new() -> Self {
        Self { buffer: String::new() }
    }
}
