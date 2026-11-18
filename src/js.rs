use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{ExportDefaultDeclarationKind, Expression, Statement};
use oxc_codegen::Codegen;
use oxc_parser::{ParseOptions, Parser};
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType, Span};
use oxc_transformer::{TransformOptions, Transformer};

fn strip_fences(src: &str) -> &str {
    let mut s = src.trim();
    if s.starts_with("```") {
        s = s.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
        if let Some(body) = s.trim_end().strip_suffix("```") {
            s = body;
        }
    }
    s.trim()
}

fn text(src: &str, span: Span) -> &str {
    &src[span.start as usize..span.end as usize]
}

fn is_function(e: &Expression) -> bool {
    matches!(e.without_parentheses(), Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_))
}

fn wrap_body(src: &str, last_expr: Option<Span>) -> String {
    let body = match last_expr {
        Some(span) => {
            format!("{}return ({});{}", &src[..span.start as usize], text(src, span), &src[span.end as usize..])
        }
        None => src.to_string(),
    };
    format!("async () => {{\n{body}\n}}")
}

pub fn normalize_code(src: &str) -> String {
    let src = strip_fences(src);
    let allocator = Allocator::default();
    let options = ParseOptions { allow_return_outside_function: true, ..ParseOptions::default() };
    let parsed = Parser::new(&allocator, src, SourceType::ts()).with_options(options).parse();
    if parsed.panicked || !parsed.diagnostics.is_empty() {
        return src.to_string();
    }
    let body = &parsed.program.body;
    if body.len() == 1 {
        match &body[0] {
            Statement::ExpressionStatement(es) if is_function(&es.expression) => {
                return text(src, es.expression.span()).to_string();
            }
            Statement::FunctionDeclaration(f) if f.id.is_some() => {
                return format!("({})", text(src, f.span));
            }
            Statement::ExportDefaultDeclaration(ed) => match &ed.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                    return format!("({})", text(src, f.span));
                }
                kind => {
                    if let Some(e) = kind.as_expression() {
                        let inner = text(src, e.span());
                        return if is_function(e) {
                            inner.to_string()
                        } else {
                            wrap_body(inner, Some(Span::new(0, inner.len() as u32)))
                        };
                    }
                }
            },
            _ => {}
        }
    }
    let last = match body.last() {
        Some(Statement::ExpressionStatement(es)) => Some(es.expression.span()),
        _ => None,
    };
    wrap_body(src, last)
}

fn describe_error(source: &str, d: &oxc_diagnostics::OxcDiagnostic) -> String {
    let mut msg = d.message.to_string();
    if let Some(label) = d.labels.first() {
        let at = (label.offset() as usize).min(source.len());
        let start = source[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let end = source[at..].find('\n').map(|i| at + i).unwrap_or(source.len());
        let line = source[start..end].trim();
        if !line.is_empty() {
            let line: String = line.chars().take(160).collect();
            msg.push_str(&format!(" at `{line}`"));
        }
    }
    if let Some(help) = &d.help {
        msg.push_str(&format!(" ({help})"));
    }
    msg
}

pub fn transpile(normalized: &str) -> Result<String, String> {
    let wrapped = format!("globalThis.__main = (\n{normalized}\n);");
    let allocator = Allocator::default();
    let source_type = SourceType::ts().with_script(true);
    let parsed = Parser::new(&allocator, &wrapped, source_type).parse();
    if let Some(d) = parsed.diagnostics.first() {
        return Err(format!("SyntaxError: {}", describe_error(&wrapped, d)));
    }
    let mut program = parsed.program;
    let scoping = SemanticBuilder::new().with_enum_eval(true).build(&program).semantic.into_scoping();
    let ret = Transformer::new(&allocator, Path::new("execute.ts"), &TransformOptions::default())
        .build_with_scoping(scoping, &mut program);
    if let Some(d) = ret.diagnostics.first() {
        return Err(format!("SyntaxError: {}", describe_error(&wrapped, d)));
    }
    Ok(Codegen::new().build(&program).code)
}

pub fn prepare(code: &str) -> Result<String, String> {
    transpile(&normalize_code(code))
}
