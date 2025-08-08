use crate::core::mode::Position;
use crossterm::{
    ExecutableCommand, QueueableCommand, cursor,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::{debug, info};
use std::io::{self, Stdout, Write};

pub struct Terminal {
    stdout: Stdout,
    size: (u16, u16), // (width, height)
}

impl Terminal {
    pub fn new() -> io::Result<Self> {
        info!("Initializing terminal with alternate screen and raw mode");
        let mut stdout = io::stdout();

        // Enter alternate screen before enabling raw mode
        stdout.execute(EnterAlternateScreen)?;
        debug!("Entered alternate screen mode");

        terminal::enable_raw_mode()?;
        debug!("Enabled raw terminal mode");

        stdout.execute(terminal::Clear(ClearType::All))?;
        stdout.execute(cursor::Hide)?;
        debug!("Cleared screen and hid cursor");

        // Flush stdout and give terminal time to settle
        stdout.flush()?;

        let size = terminal::size()?;
        info!("Terminal initialized with size: {}x{}", size.0, size.1);

        Ok(Self { stdout, size })
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    pub fn update_size(&mut self) -> io::Result<()> {
        let old_size = self.size;
        self.size = terminal::size()?;
        if old_size != self.size {
            debug!(
                "Terminal size updated from {}x{} to {}x{}",
                old_size.0, old_size.1, self.size.0, self.size.1
            );
        }
        Ok(())
    }

    pub fn clear_screen(&mut self) -> io::Result<()> {
        self.stdout.execute(Clear(ClearType::All))?;
        Ok(())
    }

    pub fn clear_line(&mut self) -> io::Result<()> {
        self.stdout.execute(Clear(ClearType::CurrentLine))?;
        Ok(())
    }

    pub fn move_cursor(&mut self, pos: Position) -> io::Result<()> {
        self.stdout
            .execute(cursor::MoveTo(pos.col as u16, pos.row as u16))?;
        Ok(())
    }

    pub fn hide_cursor(&mut self) -> io::Result<()> {
        self.stdout.execute(cursor::Hide)?;
        Ok(())
    }

    pub fn show_cursor(&mut self) -> io::Result<()> {
        self.stdout.execute(cursor::Show)?;
        Ok(())
    }

    pub fn set_foreground_color(&mut self, color: Color) -> io::Result<()> {
        self.stdout.execute(SetForegroundColor(color))?;
        Ok(())
    }

    pub fn set_background_color(&mut self, color: Color) -> io::Result<()> {
        self.stdout.execute(SetBackgroundColor(color))?;
        Ok(())
    }

    pub fn reset_color(&mut self) -> io::Result<()> {
        self.stdout.execute(ResetColor)?;
        Ok(())
    }

    pub fn print(&mut self, text: &str) -> io::Result<()> {
        self.stdout.execute(Print(text))?;
        Ok(())
    }

    pub fn print_at(&mut self, pos: Position, text: &str) -> io::Result<()> {
        self.move_cursor(pos)?;
        self.print(text)?;
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.stdout.flush()
    }

    pub fn queue_print(&mut self, text: &str) -> io::Result<()> {
        self.stdout.queue(Print(text))?;
        Ok(())
    }

    pub fn queue_move_cursor(&mut self, pos: Position) -> io::Result<()> {
        self.stdout
            .queue(cursor::MoveTo(pos.col as u16, pos.row as u16))?;
        Ok(())
    }

    pub fn queue_set_fg_color(&mut self, color: Color) -> io::Result<()> {
        self.stdout.queue(SetForegroundColor(color))?;
        Ok(())
    }

    pub fn queue_set_bg_color(&mut self, color: Color) -> io::Result<()> {
        self.stdout.queue(SetBackgroundColor(color))?;
        Ok(())
    }

    pub fn queue_reset_color(&mut self) -> io::Result<()> {
        self.stdout.queue(ResetColor)?;
        Ok(())
    }

    pub fn queue_clear_line(&mut self) -> io::Result<()> {
        self.stdout.queue(Clear(ClearType::CurrentLine))?;
        Ok(())
    }

    pub fn queue_clear_screen(&mut self) -> io::Result<()> {
        self.stdout.queue(Clear(ClearType::All))?;
        Ok(())
    }

    pub fn queue_hide_cursor(&mut self) -> io::Result<()> {
        self.stdout.queue(cursor::Hide)?;
        Ok(())
    }

    pub fn queue_show_cursor(&mut self) -> io::Result<()> {
        self.stdout.queue(cursor::Show)?;
        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        debug!("Cleaning up terminal: restoring cursor and colors");
        // Restore cursor and colors first
        let _ = self.stdout.execute(cursor::Show);
        let _ = self.stdout.execute(ResetColor);

        debug!("Disabling raw terminal mode");
        // Disable raw mode before leaving alternate screen
        let _ = terminal::disable_raw_mode();

        debug!("Leaving alternate screen mode");
        // Leave alternate screen to restore original terminal content
        let _ = self.stdout.execute(LeaveAlternateScreen);
        info!("Terminal cleanup completed");
    }
}
