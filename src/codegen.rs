use crate::{
    ast::{AstNodes, Stmt, StmtId},
    bind::SymbolId,
    chunk::Code,
};

pub struct Codegen<'a> {
    ast: &'a AstNodes,
    code: Code,
    structs: Vec<SymbolId>,
    functions: Vec<SymbolId>,
}

impl<'a> Codegen<'a> {
    pub fn new(ast: &'a AstNodes) -> Self {
        Self {
            ast,
            code: Code::new(),
            structs: Vec::new(),
            functions: Vec::new(),
        }
    }

    pub fn generate(&mut self, stmts: &[StmtId]) {
        self.collect_structs(stmts);
    }

    fn collect_structs(&mut self, stmts: &[StmtId]) {
        for &stmt in stmts {
            match self.ast.stmts[stmt] {
                Stmt::Struct(..) => {
                    let symbol = self.ast.stmts.binding(stmt);
                    self.structs.push(symbol);
                }
                _ => (),
            }
        }
    }
}
