//! Oxc Codegen
//!
//! Supports
//!
//! * whitespace removal
//! * sourcemaps
//!
//! Code adapted from
//! * [esbuild](https://github.com/evanw/esbuild/blob/main/internal/js_printer/js_printer.go)

mod annotation_comment;
mod context;
mod gen;
mod gen_ts;
mod operator;

use std::str::from_utf8_unchecked;

#[allow(clippy::wildcard_imports)]
use oxc_ast::ast::*;
use oxc_ast::{Comment, Trivias, TriviasMap};
use oxc_span::Atom;
use oxc_syntax::{
    identifier::is_identifier_part,
    operator::{BinaryOperator, UnaryOperator, UpdateOperator},
    precedence::Precedence,
    symbol::SymbolId,
};

pub use crate::{
    context::Context,
    gen::{Gen, GenExpr},
    operator::Operator,
};
// use crate::mangler::Mangler;

#[derive(Debug, Default, Clone, Copy)]
pub struct CodegenOptions {
    pub enable_typescript: bool,
    pub preserve_annotate_comments: bool,
}

pub struct Codegen<'a, const MINIFY: bool> {
    #[allow(unused)]
    options: CodegenOptions,

    // mangler: Option<Mangler>,
    /// Output Code
    code: Vec<u8>,

    // states
    prev_op_end: usize,
    prev_reg_exp_end: usize,
    need_space_before_dot: usize,

    /// For avoiding `;` if the previous statement ends with `}`.
    needs_semicolon: bool,

    prev_op: Option<Operator>,

    start_of_stmt: usize,
    start_of_arrow_expr: usize,
    start_of_default_export: usize,

    /// Track the current indentation level
    indentation: u8,
    comment_gen_related: Option<CommentGenRelated<'a>>,
}

#[derive(Debug, Clone, Copy)]
pub enum Separator {
    Comma,
    Semicolon,
    None,
}

pub struct CommentGenRelated<'a> {
    pub trivials: TriviasMap,
    pub source_code: &'a str,
    /// The key of map is the node start position,
    /// the first element of value is the start of the comment
    /// the second element of value includes the end of the comment and comment kind.
    pub move_comment_map: rustc_hash::FxHashMap<u32, (u32, Comment)>,
}

impl<'a, const MINIFY: bool> Codegen<'a, MINIFY> {
    pub fn new(
        source_len: usize,
        options: CodegenOptions,
        comment_gen_related: Option<CommentGenRelated<'a>>,
    ) -> Self {
        // Initialize the output code buffer to reduce memory reallocation.
        // Minification will reduce by at least half of the original size.
        let capacity = if MINIFY { source_len / 2 } else { source_len };
        Self {
            options,
            // mangler: None,
            code: Vec::with_capacity(capacity),
            needs_semicolon: false,
            need_space_before_dot: 0,
            prev_op_end: 0,
            prev_reg_exp_end: 0,
            prev_op: None,
            start_of_stmt: 0,
            start_of_arrow_expr: 0,
            start_of_default_export: 0,
            indentation: 0,
            comment_gen_related,
        }
    }

    // fn with_mangler(&mut self, mangler: Mangler) {
    // self.mangler = Some(mangler);
    // }

    pub fn build(mut self, program: &Program<'_>) -> String {
        program.gen(&mut self, Context::default());
        self.into_code()
    }

    pub fn into_code(self) -> String {
        // SAFETY: criteria of `from_utf8_unchecked`.are met.
        unsafe { String::from_utf8_unchecked(self.code) }
    }

    fn code(&self) -> &Vec<u8> {
        &self.code
    }

    fn code_len(&self) -> usize {
        self.code().len()
    }

    /// Push a single character into the buffer
    pub fn print(&mut self, ch: u8) {
        self.code.push(ch);
    }

    /// Push a string into the buffer
    pub fn print_str<T: AsRef<[u8]>>(&mut self, s: T) {
        self.code.extend_from_slice(s.as_ref());
    }

    /// In some scenario, we want to move the comment that should be codegened to another position.
    /// ```js
    ///  /* @__NO_SIDE_EFFECTS__ */ export const a = function() {
    ///
    ///  }, b = 10000;
    ///
    /// ```
    /// should generate such output:
    /// ```js
    ///   export const /* @__NO_SIDE_EFFECTS__ */ a = function() {
    ///
    ///  }, b = 10000;
    /// ```
    pub fn move_comment(&mut self, postion: u32, full_comment_info: (u32, Comment)) {
        if let Some(comment_gen_related) = &mut self.comment_gen_related {
            comment_gen_related.move_comment_map.insert(postion, full_comment_info);
        }
    }

    pub fn try_get_leading_comment(&self, start: u32) -> Option<(&u32, &Comment)> {
        self.comment_gen_related.as_ref().and_then(|comment_gen_related| {
            comment_gen_related.trivials.comments().range(0..start).rev().next()
        })
    }

    pub fn try_get_sourcecode(&self) -> Option<&str> {
        self.comment_gen_related
            .as_ref()
            .and_then(|comment_gen_related| Some(comment_gen_related.source_code))
    }

    pub fn try_take_moved_comment(&mut self, node_start: u32) -> Option<(u32, Comment)> {
        self.comment_gen_related.as_mut().and_then(|comment_gen_related| {
            comment_gen_related.move_comment_map.remove(&node_start)
        })
    }

    pub fn try_get_leading_comment_from_move_map(&self, start: u32) -> Option<&(u32, Comment)> {
        self.comment_gen_related
            .as_ref()
            .and_then(|comment_gen_related| comment_gen_related.move_comment_map.get(&start))
    }

    fn print_soft_space(&mut self) {
        if !MINIFY {
            self.print(b' ');
        }
    }

    pub fn print_hard_space(&mut self) {
        self.print(b' ');
    }

    fn print_soft_newline(&mut self) {
        if !MINIFY {
            self.print(b'\n');
        }
    }

    fn print_semicolon(&mut self) {
        self.print(b';');
    }

    fn print_comma(&mut self) {
        self.print(b',');
    }

    fn print_space_before_identifier(&mut self) {
        if self
            .peek_nth(0)
            .is_some_and(|ch| is_identifier_part(ch) || self.prev_reg_exp_end == self.code.len())
        {
            self.print_hard_space();
        }
    }

    fn peek_nth(&self, n: usize) -> Option<char> {
        // SAFETY: criteria of `from_utf8_unchecked`.are met.
        unsafe { from_utf8_unchecked(self.code()) }.chars().nth_back(n)
    }

    fn indent(&mut self) {
        if !MINIFY {
            self.indentation += 1;
        }
    }

    fn dedent(&mut self) {
        if !MINIFY {
            self.indentation -= 1;
        }
    }

    fn print_indent(&mut self) {
        if !MINIFY {
            for _ in 0..self.indentation {
                self.print(b'\t');
            }
        }
    }

    fn print_semicolon_after_statement(&mut self) {
        if MINIFY {
            self.needs_semicolon = true;
        } else {
            self.print_str(b";\n");
        }
    }

    fn print_semicolon_if_needed(&mut self) {
        if self.needs_semicolon {
            self.print_semicolon();
            self.needs_semicolon = false;
        }
    }

    fn print_ellipsis(&mut self) {
        self.print_str(b"...");
    }

    pub fn print_colon(&mut self) {
        self.print(b':');
    }

    fn print_equal(&mut self) {
        self.print(b'=');
    }

    fn print_sequence<T: Gen<MINIFY>>(&mut self, items: &[T], separator: Separator, ctx: Context) {
        let len = items.len();
        for (index, item) in items.iter().enumerate() {
            item.gen(self, ctx);
            match separator {
                Separator::Semicolon => self.print_semicolon(),
                Separator::Comma => self.print(b','),
                Separator::None => {}
            }
            if index != len - 1 {}
        }
    }

    fn print_block_start(&mut self) {
        self.print(b'{');
        self.print_soft_newline();
        self.indent();
    }

    fn print_block_end(&mut self) {
        self.dedent();
        self.print_indent();
        self.print(b'}');
    }

    fn print_block1(&mut self, stmt: &BlockStatement<'_>, ctx: Context) {
        self.print_block_start();
        self.print_directives_and_statements_with_semicolon_order(None, &stmt.body, ctx, true);
        self.print_block_end();
        self.needs_semicolon = false;
    }

    fn print_block<T: Gen<MINIFY>>(&mut self, items: &[T], separator: Separator, ctx: Context) {
        self.print_block_start();
        self.print_sequence(items, separator, ctx);
        self.print_block_end();
    }

    fn print_list<T: Gen<MINIFY>>(&mut self, items: &[T], ctx: Context) {
        for (index, item) in items.iter().enumerate() {
            if index != 0 {
                self.print_comma();
                self.print_soft_space();
            }
            item.gen(self, ctx);
        }
    }

    pub fn print_expression(&mut self, expr: &Expression<'_>) {
        expr.gen_expr(self, Precedence::lowest(), Context::default());
    }

    fn print_expressions<T: GenExpr<MINIFY>>(
        &mut self,
        items: &[T],
        precedence: Precedence,
        ctx: Context,
    ) {
        for (index, item) in items.iter().enumerate() {
            if index != 0 {
                self.print_comma();
            }
            item.gen_expr(self, precedence, ctx);
        }
    }

    fn print_symbol(&mut self, _symbol_id: Option<SymbolId>, fallback: &Atom) {
        // if let Some(mangler) = &self.mangler {
        // if let Some(symbol_id) = symbol_id {
        // let name = mangler.get_symbol_name(symbol_id);
        // self.print_str(name.clone().as_bytes());
        // return;
        // }
        // }
        self.print_str(fallback.as_bytes());
    }

    fn print_space_before_operator(&mut self, next: Operator) {
        if !MINIFY {
            self.print_hard_space();
            return;
        }
        if self.prev_op_end != self.code.len() {
            return;
        }
        let Some(prev) = self.prev_op else { return };
        // "+ + y" => "+ +y"
        // "+ ++ y" => "+ ++y"
        // "x + + y" => "x+ +y"
        // "x ++ + y" => "x+++y"
        // "x + ++ y" => "x+ ++y"
        // "-- >" => "-- >"
        // "< ! --" => "<! --"
        let bin_op_add = Operator::Binary(BinaryOperator::Addition);
        let bin_op_sub = Operator::Binary(BinaryOperator::Subtraction);
        let un_op_pos = Operator::Unary(UnaryOperator::UnaryPlus);
        let un_op_pre_inc = Operator::Update(UpdateOperator::Increment);
        let un_op_neg = Operator::Unary(UnaryOperator::UnaryNegation);
        let un_op_pre_dec = Operator::Update(UpdateOperator::Decrement);
        let un_op_post_dec = Operator::Update(UpdateOperator::Decrement);
        let bin_op_gt = Operator::Binary(BinaryOperator::GreaterThan);
        let un_op_not = Operator::Unary(UnaryOperator::LogicalNot);
        if ((prev == bin_op_add || prev == un_op_pos)
            && (next == bin_op_add || next == un_op_pos || next == un_op_pre_inc))
            || ((prev == bin_op_sub || prev == un_op_neg)
                && (next == bin_op_sub || next == un_op_neg || next == un_op_pre_dec))
            || (prev == un_op_post_dec && next == bin_op_gt)
            || (prev == un_op_not && next == un_op_pre_dec && self.peek_nth(1) == Some('<'))
        {
            self.print_hard_space();
        }
    }

    fn wrap<F: FnMut(&mut Self)>(&mut self, wrap: bool, mut f: F) {
        if wrap {
            self.print(b'(');
        }
        f(self);
        if wrap {
            self.print(b')');
        }
    }

    fn wrap_quote<F: FnMut(&mut Self, char)>(&mut self, s: &str, mut f: F) {
        let quote = choose_quote(s);
        self.print(quote as u8);
        f(self, quote);
        self.print(quote as u8);
    }

    fn print_directives_and_statements_with_semicolon_order(
        &mut self,
        directives: Option<&[Directive]>,
        statements: &[Statement<'_>],
        ctx: Context,
        print_semicolon_first: bool,
    ) {
        if let Some(directives) = directives {
            if directives.is_empty() {
                if let Some(Statement::ExpressionStatement(s)) = statements.first() {
                    if matches!(s.expression.get_inner_expression(), Expression::StringLiteral(_)) {
                        self.print_semicolon();
                    }
                }
            } else {
                for directive in directives {
                    directive.gen(self, ctx);
                }
                self.print_soft_newline();
            }
        }
        for stmt in statements {
            if let Statement::Declaration(decl) = stmt {
                if decl.is_typescript_syntax()
                    && !self.options.enable_typescript
                    && !matches!(decl, Declaration::TSEnumDeclaration(_))
                {
                    continue;
                }
            }
            if print_semicolon_first {
                self.print_semicolon_if_needed();
                stmt.gen(self, ctx);
            } else {
                stmt.gen(self, ctx);
                self.print_semicolon_if_needed();
            }
        }
    }
}

fn choose_quote(s: &str) -> char {
    let mut single_cost = 0;
    let mut double_cost = 0;
    for c in s.chars() {
        match c {
            '\'' => single_cost += 1,
            '"' => double_cost += 1,
            _ => {}
        }
    }

    if single_cost > double_cost {
        '"'
    } else {
        '\''
    }
}
