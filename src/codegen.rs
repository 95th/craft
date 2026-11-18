use crate::{
    ast::{AstNodes, AstVisitor, BinaryOp, Expr, ExprId, Stmt, StmtId},
    bind::SymbolId,
    chunk::{Code, Opcode},
    typechecker::TyContext,
};

pub struct Codegen<'a> {
    ast: &'a AstNodes,
    ty_ctx: TyContext,
    code: Code,
}

impl<'a> Codegen<'a> {
    pub fn new(ast: &'a AstNodes, ty_ctx: TyContext) -> Self {
        Self {
            ast,
            ty_ctx,
            code: Code::new(),
        }
    }

    pub fn generate(mut self, stmts: &[StmtId]) -> Code {
        for &stmt in stmts {
            self.process_stmt(stmt);
        }
        self.code.write(Opcode::Return);
        self.code
    }

    fn process_stmt(&mut self, stmt: StmtId) {
        match self.ast.stmts[stmt] {
            Stmt::Let(_, _, init) => self.process_expr(init),
            Stmt::Expr(expr, _) => {
                self.process_expr(expr);
                self.code.pop();
            }
            _ => {}
        }
    }

    fn process_expr(&mut self, expr: ExprId) {
        match self.ast.exprs[expr] {
            Expr::Bool(b) => self
                .code
                .write(if b { Opcode::True } else { Opcode::False }),
            Expr::Number(n) => {
                self.code.write(Opcode::Const);
                self.code.write_u64(n.to_bits());
            }
            Expr::Not(expr) => {
                self.process_expr(expr);
                self.code.write(Opcode::Not)
            }
            Expr::Negate(expr) => {
                self.process_expr(expr);
                self.code.write(Opcode::Negate)
            }
            Expr::Variable(_) => {
                let symbol = self.ast.exprs.binding(expr);
                let index = self.ty_ctx.bind_ctx.symbols[symbol].local_index;
                if index <= u8::MAX as u32 {
                    self.code.write(Opcode::GetLocal);
                    self.code.write_u8(index as u8);
                } else {
                    self.code.write(Opcode::GetLocalLong);
                    self.code.write_u16(index as u16);
                }
            }
            Expr::Assign(lhs, rhs) => {
                let symbol = self.ast.exprs.binding(lhs);
                let index = self.ty_ctx.bind_ctx.symbols[symbol].local_index;
                self.process_expr(rhs);
                if index <= u8::MAX as u32 {
                    self.code.write(Opcode::SetLocal);
                    self.code.write_u8(index as u8);
                } else {
                    self.code.write(Opcode::SetLocalLong);
                    self.code.write_u16(index as u16);
                }
                self.code.write(Opcode::Unit);
            }
            Expr::Binary(lhs, op, rhs) => match op {
                BinaryOp::Or | BinaryOp::And => todo!(),
                _ => {
                    self.process_expr(lhs);
                    self.process_expr(rhs);
                    match op {
                        BinaryOp::Add => self.code.write(Opcode::Add),
                        BinaryOp::Sub => self.code.write(Opcode::Sub),
                        BinaryOp::Mul => self.code.write(Opcode::Mul),
                        BinaryOp::Div => self.code.write(Opcode::Div),
                        BinaryOp::Greater => self.code.write(Opcode::Greater),
                        BinaryOp::GreaterEqual => self.code.write(Opcode::GreaterEqual),
                        BinaryOp::Less => self.code.write(Opcode::Less),
                        BinaryOp::LessEqual => self.code.write(Opcode::LessEqual),
                        BinaryOp::Equal => self.code.write(Opcode::Equal),
                        BinaryOp::NotEqual => self.code.write(Opcode::NotEqual),
                        BinaryOp::BitOr => self.code.write(Opcode::BitOr),
                        BinaryOp::BitAnd => self.code.write(Opcode::BitAnd),
                        _ => {}
                    }
                }
            },
            Expr::Call(callee, ref args) => {
                for &arg in args {
                    self.process_expr(arg);
                }
                let symbol = self.ast.exprs.binding(callee);
                if symbol == self.ty_ctx.bind_ctx.print_number_function {
                    self.code.write(Opcode::PrintNumber);
                    self.code.write(Opcode::Unit);
                } else {
                    self.process_expr(callee);
                    self.code.write(Opcode::Call);
                    self.code.write_u8(args.len() as u8);
                }
            }
            Expr::Block(ref stmts, expr) => {
                for &stmt in stmts {
                    self.process_stmt(stmt);
                }
                if let Some(expr) = expr {
                    self.process_expr(expr);
                } else {
                    self.code.write(Opcode::Unit);
                }
            }
            Expr::If(cond, then_clause, else_clause) => {
                self.process_expr(cond);
                let then_offset = self.code.write_jump(Opcode::JumpIfFalse);
                self.process_expr(then_clause);
                if let Some(else_clause) = else_clause {
                    let else_offset = self.code.write_jump(Opcode::Jump);
                    self.code.patch_jump(then_offset);
                    self.process_expr(else_clause);
                    self.code.patch_jump(else_offset);
                } else {
                    self.code.patch_jump(then_offset);
                }
            }
            Expr::While(cond, body) => {
                let start = self.code.len();
                self.process_expr(cond);
                let jump_offset = self.code.write_jump(Opcode::JumpIfFalse);
                self.process_expr(body);
                self.code.pop();
                self.code.write_loop(start);
                self.code.patch_jump(jump_offset);
            }
            _ => {}
        }
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
