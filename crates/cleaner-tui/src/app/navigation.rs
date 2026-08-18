use super::App;
use std::ffi::OsStr;

impl App {
    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    pub fn move_down(&mut self) {
        if self.selected < self.entries.len().saturating_sub(1) {
            self.selected += 1;
        }
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    pub fn go_top(&mut self) {
        self.selected = 0;
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    pub fn go_bottom(&mut self) {
        self.selected = self.entries.len().saturating_sub(1);
        self.confirm_delete = false;
        self.confirm_clean = false;
    }

    pub fn enter(&mut self) {
        if self.is_busy() {
            return;
        }
        if let Some(entry) = self.entries.get(self.selected) {
            let is_dir = entry.is_dir;
            let name = entry.name.clone();
            if is_dir {
                if name == ".." {
                    self.go_back();
                } else {
                    self.path_stack.push(self.current_path.clone());
                    self.current_path.push(name);
                    self.load_current_dir();
                }
            }
        }
    }

    pub fn go_back(&mut self) {
        if self.is_busy() {
            return;
        }
        if let Some(prev) = self.path_stack.pop() {
            // Get current folder name to restore cursor position
            let current_name = self.current_path.file_name().map(OsStr::to_os_string);

            self.current_path = prev;
            self.load_current_dir_with_selection(current_name.as_deref());
        }
        self.confirm_delete = false;
        self.confirm_clean = false;
    }
}
