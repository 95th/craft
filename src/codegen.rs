use rustc_hash::FxHashMap;

use crate::{
    ast::{AstNodes, AstVisitor, BinaryOp, Expr, ExprId, Stmt, StmtId},
    bind::{SymbolId, SymbolKind},
    chunk::{Code, Opcode},
    heap::{Heap, ObjKind},
    typechecker::TyContext,
    types::Ty,
};

struct LoopCtx {
    offset: usize,
    break_offsets: Vec<usize>,
}

impl LoopCtx {
    fn new(offset: usize) -> Self {
        Self {
            offset,
            break_offsets: Vec::new(),
        }
    }
}

pub struct Codegen<'a> {
    ast: &'a AstNodes,
    ty_ctx: TyContext,
    code: Code,
    loop_ctxts: Vec<LoopCtx>,
    functions: FxHashMap<SymbolId, usize>,
    heap: &'a mut Heap,
}

impl<'a> Codegen<'a> {
    pub fn new(ast: &'a AstNodes, heap: &'a mut Heap, ty_ctx: TyContext) -> Self {
        Self {
            ast,
            ty_ctx,
            code: Code::new(),
            loop_ctxts: Vec::new(),
            functions: FxHashMap::default(),
            heap,
        }
    }

    pub fn generate(mut self, stmts: &[StmtId]) -> Code {
        let collector = NodeCollector::new(stmts, self.ast);

        // Collector runs in pre-order traversal. Iterate in
        // reverse order to process innermost functions first.
        for &function in collector.functions.iter().rev() {
            self.process_function(function);
        }

        self.code.start_main();
        for &stmt in stmts {
            self.process_stmt(stmt);
        }
        self.code.write(Opcode::Return);
        self.code
    }

    fn process_function(&mut self, function: StmtId) {
        let symbol = self.ast.stmts.binding(function);
        self.functions.insert(symbol, self.code.len());
        let Stmt::Fn(.., body) = self.ast.stmts[function] else {
            unreachable!();
        };
        self.process_expr(body);
        self.code.write(Opcode::Return);
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
            Expr::String(ref s) => {
                let obj = self.heap.alloc(ObjKind::String, s.as_bytes());
                self.code.write(Opcode::Const);
                self.code.write_u64(obj as u64);
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
                let symbol_id = self.ast.exprs.binding(expr);
                let symbol = &self.ty_ctx.bind_ctx.symbols[symbol_id];
                match symbol.kind {
                    SymbolKind::Let | SymbolKind::Param => {
                        let index = symbol.local_index;
                        if index <= u8::MAX as u32 {
                            self.code.write(Opcode::GetLocal);
                            self.code.write_u8(index as u8);
                        } else {
                            self.code.write(Opcode::GetLocalLong);
                            self.code.write_u16(index as u16);
                        }
                    }
                    SymbolKind::Fn => {
                        let offset = self.functions[&symbol_id];
                        self.code.write(Opcode::Const);
                        self.code.write_u64(offset as u64);
                    }
                    _ => unreachable!(),
                }
            }
            Expr::Assign(lhs, rhs) => match self.ast.exprs[lhs] {
                Expr::Variable(..) => {
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
                Expr::TupleField(expr, index) => {
                    self.process_expr(expr);
                    self.process_expr(rhs);
                    self.code.write(Opcode::SetStructProperty);
                    self.code.write_u16(index);
                    self.code.write(Opcode::Unit);
                }
                Expr::StructField(expr, ref name) => {
                    let ty = self.ty_ctx.expr_types[&expr];
                    let Ty::Named(symbol) = self.ty_ctx.types[ty] else {
                        unreachable!();
                    };
                    let properties = &self.ty_ctx.struct_properties[&symbol];
                    let name = self.ty_ctx.bind_ctx.interner.intern(name);
                    let index = properties.get_index_of(&name).unwrap();
                    self.process_expr(expr);
                    self.process_expr(rhs);
                    self.code.write(Opcode::SetStructProperty);
                    self.code.write_u16(index as u16);
                    self.code.write(Opcode::Unit);
                }
                _ => unreachable!(),
            },
            Expr::Binary(lhs, BinaryOp::And, rhs) => {
                self.process_expr(lhs);
                let jump_false = self.code.write_jump(Opcode::JumpIfFalse);
                self.process_expr(rhs);
                let jump_end = self.code.write_jump(Opcode::Jump);
                self.code.patch_jump(jump_false);
                self.code.write(Opcode::False);
                self.code.patch_jump(jump_end);
            }
            Expr::Binary(lhs, BinaryOp::Or, rhs) => {
                self.process_expr(lhs);
                let jump_rhs = self.code.write_jump(Opcode::JumpIfFalse);
                let jump_true = self.code.write_jump(Opcode::Jump);
                self.code.patch_jump(jump_rhs);
                self.process_expr(rhs);
                let jump_end = self.code.write_jump(Opcode::Jump);
                self.code.patch_jump(jump_true);
                self.code.write(Opcode::True);
                self.code.patch_jump(jump_end);
            }
            Expr::Binary(lhs, op, rhs) => {
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
            Expr::Call(callee, ref args) => {
                for &arg in args {
                    self.process_expr(arg);
                }
                let symbol = self.ast.exprs.binding(callee);
                if symbol == self.ty_ctx.bind_ctx.print_function {
                    let ty = self.ty_ctx.expr_types[&args[0]];
                    match self.ty_ctx.types[ty] {
                        Ty::Number => self.code.write(Opcode::PrintNumber),
                        Ty::String => self.code.write(Opcode::PrintString),
                        Ty::Bool => self.code.write(Opcode::PrintBool),
                        _ => self.code.write(Opcode::PrintObj),
                    }
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
                    self.code.write(Opcode::Unit);
                }
            }
            Expr::While(cond, body) => {
                self.enter_loop();
                let start = self.code.len();
                self.process_expr(cond);
                let jump_offset = self.code.write_jump(Opcode::JumpIfFalse);
                self.process_expr(body);
                self.code.pop();
                self.code.write_loop(start);
                self.code.patch_jump(jump_offset);
                self.exit_loop();
            }
            Expr::For(init, cond, incr, body) => {
                if let Some(init) = init {
                    self.process_stmt(init);
                }
                self.enter_loop();
                let start = self.code.len();
                if let Some(cond) = cond {
                    self.process_expr(cond);
                    let jump_offset = self.code.write_jump(Opcode::JumpIfFalse);
                    self.process_expr(body);
                    self.code.pop();
                    if let Some(incr) = incr {
                        self.process_expr(incr);
                        self.code.pop();
                    }
                    self.code.write_loop(start);
                    self.code.patch_jump(jump_offset);
                } else {
                    self.process_expr(body);
                    self.code.pop();
                    if let Some(incr) = incr {
                        self.process_expr(incr);
                        self.code.pop();
                    }
                    self.code.write_loop(start);
                }
                self.exit_loop();
            }
            Expr::Break => {
                let offset = self.code.write_jump(Opcode::Jump);
                self.push_loop_break(offset);
                self.code.pop();
            }
            Expr::Continue => {
                self.code
                    .write_loop(self.loop_ctxts.last_mut().unwrap().offset);
                self.code.pop();
            }
            Expr::Unit => self.code.write(Opcode::Unit),
            Expr::Return(expr) => {
                if let Some(expr) = expr {
                    self.process_expr(expr);
                    self.code.write(Opcode::Return);
                } else {
                    self.code.write(Opcode::Unit);
                    self.code.write(Opcode::Return);
                }
            }
            Expr::Tuple(ref args) => {
                for &arg in args {
                    self.process_expr(arg);
                }
                self.code.write(Opcode::Struct);
                self.code.write_u16(args.len() as u16);
            }
            Expr::TupleField(expr, index) => {
                self.process_expr(expr);
                self.code.write(Opcode::StructProperty);
                self.code.write_u16(index);
            }
            Expr::Struct(.., ref fields) => {
                let symbol = self.ast.exprs.binding(expr);
                let properties = &self.ty_ctx.struct_properties[&symbol];
                let mut exprs = Vec::with_capacity(properties.len());
                for (name, _) in properties {
                    let name = self.ty_ctx.bind_ctx.interner.lookup(*name);
                    exprs.push(fields[name]);
                }
                for &expr in exprs.iter() {
                    self.process_expr(expr);
                }
                self.code.write(Opcode::Struct);
                self.code.write_u16(exprs.len() as u16);
            }
            Expr::StructField(expr, ref name) => {
                let ty = self.ty_ctx.expr_types[&expr];
                let Ty::Named(symbol) = self.ty_ctx.types[ty] else {
                    unreachable!();
                };
                let properties = &self.ty_ctx.struct_properties[&symbol];
                let name = self.ty_ctx.bind_ctx.interner.intern(name);
                let index = properties.get_index_of(&name).unwrap();
                self.process_expr(expr);
                self.code.write(Opcode::StructProperty);
                self.code.write_u16(index as u16);
            }
        }
    }

    fn enter_loop(&mut self) {
        self.loop_ctxts.push(LoopCtx::new(self.code.len()));
    }

    fn push_loop_break(&mut self, offset: usize) {
        self.loop_ctxts
            .last_mut()
            .unwrap()
            .break_offsets
            .push(offset);
    }

    fn exit_loop(&mut self) {
        let ctx = self.loop_ctxts.pop().unwrap();
        for offset in ctx.break_offsets {
            self.code.patch_jump(offset);
        }
    }
}

struct NodeCollector {
    structs: Vec<StmtId>,
    functions: Vec<StmtId>,
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
            Stmt::Struct(..) => self.structs.push(stmt),
            Stmt::Fn(..) => self.functions.push(stmt),
            _ => {}
        }
    }
}
