use crate::{
    ast::{AstNodes, AstVisitor, Stmt, StmtId},
    bind::SymbolId,
    chunk::Code,
};

pub struct Codegen<'a> {
    ast: &'a AstNodes,
    code: Code,
}

impl<'a> Codegen<'a> {
    pub fn new(ast: &'a AstNodes) -> Self {
        Self {
            ast,
            code: Code::new(),
        }
    }

    pub fn generate(&mut self, stmts: &[StmtId]) {
        let nodes = NodeCollector::new(stmts, self.ast);
        println!(
            "{} structs, {} functions",
            nodes.structs.len(),
            nodes.functions.len()
        );
    }
}
struct NodeCollector {
    structs: Vec<SymbolId>,
    functions: Vec<SymbolId>,
}

impl NodeCollector {
    fn new(stmts: &[StmtId], ast: &AstNodes) -> Self {
        let mut collector = Self {
            structs: Vec::new(),
            functions: Vec::new(),
        };
        ast.visit(stmts, &mut collector);
        collector
    }
}

impl AstVisitor for NodeCollector {
    fn visit_stmt(&mut self, stmt: StmtId, ast: &AstNodes) {
        match ast.stmts[stmt] {
            Stmt::Struct(..) => self.structs.push(ast.stmts.binding(stmt)),
            Stmt::Fn(..) => self.functions.push(ast.stmts.binding(stmt)),
            _ => {}
        }
    }
}
