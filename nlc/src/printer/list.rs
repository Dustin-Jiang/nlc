//! The `nlc list [<file>]` view: a plain section-tree dump.

use super::{line, Printer};
use crate::model::Section;
use crate::snapshot::Snapshot;

/// Renders the section hierarchy of one file, or every file when `file` is
/// `None`.
pub struct ListPrinter {
    pub file: Option<String>,
}

impl Printer for ListPrinter {
    fn print(&self, s: &Snapshot, out: &mut String) -> i32 {
        let target_files: Vec<String> = match &self.file {
            Some(one) => {
                if !s.world.files.contains_key(one) {
                    line(out, &format!("nlc: no such file `{one}`"));
                    return 2;
                }
                vec![one.clone()]
            }
            None => s.world.files.keys().cloned().collect(),
        };

        for (i, path) in target_files.iter().enumerate() {
            if i > 0 {
                line(out, "");
            }
            let f = s.world.file(path).unwrap();
            line(out, path);
            if !f.preamble.is_empty() {
                line(out, &format!("  (preamble: {} block(s))", f.preamble.len()));
            }
            for section in &f.sections {
                print_section(section, 1, out);
            }
        }
        0
    }
}

fn print_section(s: &Section, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let title = s.title();
    line(out, &format!("{}{} {title}", indent, "#".repeat(s.level as usize)));
    if !s.body.is_empty() {
        line(out, &format!("{indent}  ({} block(s))", s.body.len()));
    }
    for child in &s.children {
        print_section(child, depth + 1, out);
    }
}
