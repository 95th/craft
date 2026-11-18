use crate::ast::Span;

struct Diagnostic {
    message: String,
    span: Span,
}

pub struct Diagnostics {
    diagnostics: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self {
            diagnostics: Vec::new(),
        }
    }

    pub fn push(&mut self, message: impl Into<String>, span: Span) {
        self.diagnostics.push(Diagnostic {
            message: message.into(),
            span,
        })
    }

    pub fn print(&self, source: &str) {
        let lines = build_lines(source);
        for d in self.diagnostics.iter() {
            let start = find_line(&lines, d.span.start as usize);
            let end = find_line(&lines, d.span.end as usize);
            let mut line_number = start + 1;
            let mut i = d.span.start as usize;
            for &(start_index, line) in lines[start..=end].iter() {
                println!();
                println!("{line_number:5} | {line}");
                print!("      | ");
                for _ in start_index..i {
                    print!(" ");
                }
                for _ in i..(d.span.end as usize).min(start_index + line.len()) {
                    print!("^");
                }
                i = start_index + line.len() + 1;
                line_number += 1;
            }
            println!(" {}", d.message);
        }
    }

    pub fn len(&self) -> usize {
        self.diagnostics.len()
    }

    pub fn truncate(&mut self, len: usize) {
        self.diagnostics.truncate(len);
    }
}

fn build_lines(source: &str) -> Vec<(usize, &str)> {
    let mut lines = Vec::new();
    let mut i = 0;
    for line in source.split('\n') {
        lines.push((i, line));
        i += line.len() + 1;
    }
    lines
}

fn find_line(lines: &[(usize, &str)], index: usize) -> usize {
    match lines.binary_search_by_key(&index, |&(i, _)| i) {
        Ok(i) => i,
        Err(i) => i - 1,
    }
}
