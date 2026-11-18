use crate::ast::Span;

pub struct Diagnostic {
    pub message: String,
    pub span: Span,
}

pub fn print_diagnostics(source: &str, diagnostics: Vec<Diagnostic>) {
    let lines = build_lines(source);
    for d in diagnostics {
        let start = find_line(&lines, d.span.start as usize);
        let end = find_line(&lines, d.span.end as usize);
        println!("```");
        let mut line_number = start + 1;
        let mut i = d.span.start as usize;
        for &(start_index, line) in lines[start..=end].iter() {
            println!("{line_number:5} | {line}");
            print!("      | ");
            for _ in start_index..i {
                print!(" ");
            }
            for _ in i..(d.span.end as usize).min(start_index + line.len()) {
                print!("^");
            }
            i = start_index + line.len() + 1;
            println!();
            line_number += 1;
        }
        println!("```");
        println!("Error: {}", d.message);
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
