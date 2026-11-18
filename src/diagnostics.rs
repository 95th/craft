use crate::ast::Span;

pub struct Diagnostic {
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn print(&self, source: &str) {
        println!("Error at '{}': {}", self.span.text(source), self.message);
    }
}
