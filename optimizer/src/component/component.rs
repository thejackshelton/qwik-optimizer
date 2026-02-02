use crate::collector::{ExportInfo, Id as CollectorId};
use crate::component::*;
use crate::segment::Segment;
use crate::transform::TransformOptions;
use crate::{component::Language, import_clean_up::ImportCleanUp};
use oxc_allocator::{Allocator, Box as OxcBox, CloneIn, IntoIn, Vec as OxcVec};
use oxc_ast::ast::*;
use oxc_ast::*;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_minifier::*;
use oxc_parser;
use oxc_span::{SourceType, SPAN};
use serde::Serialize;

/// A lazy-loadable QRL segment with generated code and metadata.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct QrlComponent {
    pub id: Id,
    pub language: Language,
    pub code: String,
    pub qrl: Qrl,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_data: Option<SegmentData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}

impl QrlComponent {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        options: &TransformOptions,
        source_info: &SourceInfo,
        id: Id,
        exported_expression: Expression<'_>,
        imports: Vec<Import>,
        qrl_type: QrlType,
        segment_data: Option<SegmentData>,
        entry: Option<String>,
    ) -> QrlComponent {
        Self::new_with_hoisted_imports(
            options,
            source_info,
            id,
            exported_expression,
            imports,
            Vec::new(), // No hoisted imports
            qrl_type,
            segment_data,
            entry,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_hoisted_imports(
        options: &TransformOptions,
        source_info: &SourceInfo,
        id: Id,
        exported_expression: Expression<'_>,
        imports: Vec<Import>,
        hoisted_imports: Vec<(String, String)>,
        qrl_type: QrlType,
        segment_data: Option<SegmentData>,
        entry: Option<String>,
    ) -> QrlComponent {
        Self::new_with_hoisted(
            options,
            source_info,
            id,
            exported_expression,
            imports,
            hoisted_imports,
            Vec::new(), // No hoisted functions
            qrl_type,
            segment_data,
            entry,
        )
    }

    /// Creates a new QrlComponent with both hoisted imports and hoisted functions.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_hoisted(
        options: &TransformOptions,
        source_info: &SourceInfo,
        id: Id,
        exported_expression: Expression<'_>,
        imports: Vec<Import>,
        hoisted_imports: Vec<(String, String)>,
        hoisted_fns: Vec<(String, String, String)>,
        qrl_type: QrlType,
        segment_data: Option<SegmentData>,
        entry: Option<String>,
    ) -> QrlComponent {
        // Delegate to new_with_hoisted_all with empty hoisted_qrls
        Self::new_with_hoisted_all(
            options,
            source_info,
            id,
            exported_expression,
            imports,
            hoisted_imports,
            hoisted_fns,
            Vec::new(), // No hoisted QRLs
            qrl_type,
            segment_data,
            entry,
        )
    }

    /// Creates a new QrlComponent with hoisted imports, hoisted functions, AND hoisted QRLs.
    /// Hoisted QRLs are const declarations that go INSIDE the function body (before return).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_hoisted_all(
        options: &TransformOptions,
        source_info: &SourceInfo,
        id: Id,
        exported_expression: Expression<'_>,
        imports: Vec<Import>,
        hoisted_imports: Vec<(String, String)>,
        hoisted_fns: Vec<(String, String, String)>,
        hoisted_qrls: Vec<(String, String)>,
        qrl_type: QrlType,
        segment_data: Option<SegmentData>,
        entry: Option<String>,
    ) -> QrlComponent {
        let language = source_info.language.clone();

        let scoped_idents: Vec<CollectorId> = segment_data
            .as_ref()
            .map(|d| d.scoped_idents.clone())
            .unwrap_or_default();

        let referenced_exports: Vec<ExportInfo> = segment_data
            .as_ref()
            .map(|d| d.referenced_exports.clone())
            .unwrap_or_default();

        let iteration_params: Vec<CollectorId> = segment_data
            .as_ref()
            .map(|d| d.iteration_params.clone())
            .unwrap_or_default();

        let qrl = Qrl::new_with_iteration_params(
            &id.local_file_name,
            &id.symbol_name,
            qrl_type,
            scoped_idents.clone(),
            referenced_exports.clone(),
            iteration_params.clone(),
        );

        let source_file_imports = Self::generate_source_file_imports(&referenced_exports, source_info);

        let mut all_imports = imports;
        all_imports.extend(source_file_imports);

        // Add useLexicalScope import for any segment that has lexical captures (scoped_idents)
        if !scoped_idents.is_empty() {
            all_imports.push(Import::use_lexical_scope());
        }

        // Add qrl import if there are hoisted QRLs
        if !hoisted_qrls.is_empty() {
            all_imports.push(Import::qrl());
        }

        let source_type: SourceType = language.into();

        let code = Self::gen_with_hoisted_qrls(
            options,
            &id,
            exported_expression,
            all_imports,
            hoisted_imports,
            hoisted_fns,
            hoisted_qrls,
            &source_type,
            source_info,
            &scoped_idents,
            &iteration_params,
            &Allocator::default(),
        );
        QrlComponent {
            id,
            language: source_info.language.clone(),
            code,
            qrl,
            segment_data,
            entry,
        }
    }

    fn generate_source_file_imports(
        referenced_exports: &[ExportInfo],
        source_info: &SourceInfo,
    ) -> Vec<Import> {
        if referenced_exports.is_empty() {
            return Vec::new();
        }

        let source_file_name = source_info
            .rel_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("index");
        let source_path = format!("./{}", source_file_name);

        referenced_exports
            .iter()
            .map(|export| {
                let import_id = if export.is_default {
                    ImportId::NamedWithAlias("default".to_string(), export.local_name.clone())
                } else if export.exported_name != export.local_name {
                    ImportId::NamedWithAlias(export.exported_name.clone(), export.local_name.clone())
                } else {
                    ImportId::Named(export.local_name.clone())
                };

                Import::new(vec![import_id], &source_path)
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn gen(
        options: &TransformOptions,
        id: &Id,
        exported_expression: Expression<'_>,
        imports: Vec<Import>,
        hoisted_imports: Vec<(String, String)>,
        hoisted_fns: Vec<(String, String, String)>,
        source_type: &SourceType,
        _source_info: &SourceInfo,
        scoped_idents: &[CollectorId],
        iteration_params: &[CollectorId],
        allocator: &Allocator,
    ) -> String {
        use crate::code_move::transform_function_with_params;

        let name = &id.symbol_name;

        let ast_builder = AstBuilder::new(allocator);

        let transformed_expression = if !scoped_idents.is_empty() || !iteration_params.is_empty() {
            transform_function_with_params(exported_expression, scoped_idents, iteration_params, allocator)
        } else {
            exported_expression
        };

        let bind_pat = ast_builder.binding_pattern_binding_identifier(SPAN, name.as_str());
        let mut var_declarator = OxcVec::new_in(allocator);

        var_declarator.push(ast_builder.variable_declarator(
            SPAN,
            VariableDeclarationKind::Const,
            bind_pat,
            None::<OxcBox<'_, TSTypeAnnotation<'_>>>,
            Some(transformed_expression),
            false,
        ));

        let decl = ast_builder.variable_declaration(
            SPAN,
            VariableDeclarationKind::Const,
            var_declarator,
            false,
        );
        let decl = OxcBox::new_in(decl, allocator);
        let decl = Declaration::VariableDeclaration(decl);
        let export = ast_builder.export_named_declaration(
            SPAN,
            Some(decl),
            OxcVec::new_in(allocator),
            None,
            ImportOrExportKind::Value,
            None::<OxcBox<WithClause>>,
        );
        let export = Statement::ExportNamedDeclaration(OxcBox::new_in(export, allocator));

        let imports = imports.iter().map(|import| {
            let statement: Statement = import.clone().into_in(allocator);
            statement
        });

        let mut body = ast_builder.vec_from_iter(imports);

        // Generate hoisted import declarations for child QRLs
        // Format: const i_{hash} = ()=>import("./file");
        for (ident_name, filename) in hoisted_imports.iter().rev() {
            let hoisted_stmt = Statement::VariableDeclaration(ast_builder.alloc(ast_builder.variable_declaration(
                SPAN,
                VariableDeclarationKind::Const,
                ast_builder.vec1(ast_builder.variable_declarator(
                    SPAN,
                    VariableDeclarationKind::Const,
                    ast_builder.binding_pattern_binding_identifier(SPAN, ast_builder.atom(ident_name)),
                    None::<OxcBox<'_, TSTypeAnnotation<'_>>>,
                    Some(ast_builder.expression_arrow_function(
                        SPAN,
                        true,
                        false,
                        None::<OxcBox<TSTypeParameterDeclaration>>,
                        ast_builder.formal_parameters(
                            SPAN,
                            FormalParameterKind::ArrowFormalParameters,
                            ast_builder.vec(),
                            NONE,
                        ),
                        None::<OxcBox<TSTypeAnnotation>>,
                        ast_builder.function_body(
                            SPAN,
                            ast_builder.vec(),
                            ast_builder.vec1(ast_builder.statement_expression(
                                SPAN,
                                ast_builder.expression_import(
                                    SPAN,
                                    ast_builder.expression_string_literal(SPAN, ast_builder.atom(filename), None),
                                    None,
                                    None,
                                ),
                            )),
                        ),
                    )),
                    false,
                )),
                false,
            )));
            body.push(hoisted_stmt);
        }

        // Generate hoisted function declarations for _fnSignal
        // Format: const _hf0 = (p0)=>...; const _hf0_str = "...";
        for (hf_name, hf_code, hf_str) in hoisted_fns.iter() {
            // Parse the serialized function code back to an Expression
            let hf_allocator = Allocator::default();
            let parsed = oxc_parser::Parser::new(&hf_allocator, hf_code, SourceType::default())
                .parse_expression();
            if let Ok(hf_expr) = parsed {
                let hf_expr_cloned = hf_expr.clone_in(allocator);

                // const _hf0 = (p0)=>...;
                let hf_stmt = Statement::VariableDeclaration(ast_builder.alloc(ast_builder.variable_declaration(
                    SPAN,
                    VariableDeclarationKind::Const,
                    ast_builder.vec1(ast_builder.variable_declarator(
                        SPAN,
                        VariableDeclarationKind::Const,
                        ast_builder.binding_pattern_binding_identifier(SPAN, ast_builder.atom(hf_name)),
                        None::<OxcBox<'_, TSTypeAnnotation<'_>>>,
                        Some(hf_expr_cloned),
                        false,
                    )),
                    false,
                )));
                body.push(hf_stmt);

                // const _hf0_str = "...";
                let hf_str_name = format!("{}_str", hf_name);
                let hf_str_stmt = Statement::VariableDeclaration(ast_builder.alloc(ast_builder.variable_declaration(
                    SPAN,
                    VariableDeclarationKind::Const,
                    ast_builder.vec1(ast_builder.variable_declarator(
                        SPAN,
                        VariableDeclarationKind::Const,
                        ast_builder.binding_pattern_binding_identifier(SPAN, ast_builder.atom(&hf_str_name)),
                        None::<OxcBox<'_, TSTypeAnnotation<'_>>>,
                        Some(ast_builder.expression_string_literal(SPAN, ast_builder.atom(hf_str), None)),
                        false,
                    )),
                    false,
                )));
                body.push(hf_str_stmt);
            }
        }

        body.push(export);

        let ast_builder = AstBuilder::new(allocator);

        let mut new_pgm = ast_builder.program(
            SPAN,
            *source_type,
            "",
            OxcVec::new_in(allocator),
            None,
            OxcVec::new_in(allocator),
            body,
        );

        ImportCleanUp::clean_up(&mut new_pgm, allocator);

        // format_output overrides minify for whitespace purposes
        let should_minify = if options.format_output {
            false // Readable output when format_output is true
        } else {
            options.minify
        };

        let codegen = Codegen::new();
        let codegen_options = CodegenOptions {
            minify: should_minify,
            ..Default::default()
        };

        let code = if options.minify && !options.format_output {
            let ops = MinifierOptions {
                compress: Some(CompressOptions::default()),
                mangle: None,
            };
            let minifier = Minifier::new(ops);
            let ret = minifier.minify(allocator, &mut new_pgm);
            let scoping = ret.scoping;

            codegen
                .with_options(codegen_options)
                .with_scoping(scoping)
                .build(&new_pgm)
                .code
        } else {
            codegen.with_options(codegen_options).build(&new_pgm).code
        };
        // Post-process to match qwik-core format:
        // 1. PURE annotations: /* @__PURE__ */ -> /*#__PURE__*/
        // 2. Arrow function spacing: ) => -> )=>
        let code = code.replace("/* @__PURE__ */", "/*#__PURE__*/");
        code.replace(") => ", ")=>")
    }

    /// Generate segment code with hoisted QRL declarations inside the function body.
    /// Hoisted QRLs are const declarations that go INSIDE the function (before return statement).
    #[allow(clippy::too_many_arguments)]
    fn gen_with_hoisted_qrls(
        options: &TransformOptions,
        id: &Id,
        exported_expression: Expression<'_>,
        imports: Vec<Import>,
        hoisted_imports: Vec<(String, String)>,
        hoisted_fns: Vec<(String, String, String)>,
        hoisted_qrls: Vec<(String, String)>,
        source_type: &SourceType,
        source_info: &SourceInfo,
        scoped_idents: &[CollectorId],
        iteration_params: &[CollectorId],
        allocator: &Allocator,
    ) -> String {
        // If no hoisted QRLs, use the standard gen method
        if hoisted_qrls.is_empty() {
            return Self::gen(
                options,
                id,
                exported_expression,
                imports,
                hoisted_imports,
                hoisted_fns,
                source_type,
                source_info,
                scoped_idents,
                iteration_params,
                allocator,
            );
        }

        use crate::code_move::transform_function_with_params;

        let name = &id.symbol_name;
        let ast_builder = AstBuilder::new(allocator);

        // First transform the function expression (add params, useLexicalScope)
        let mut transformed_expression = if !scoped_idents.is_empty() || !iteration_params.is_empty() {
            transform_function_with_params(exported_expression, scoped_idents, iteration_params, allocator)
        } else {
            exported_expression
        };

        // Now inject hoisted QRL declarations into the function body
        match &mut transformed_expression {
            Expression::FunctionExpression(func) => {
                if let Some(ref mut body) = func.body {
                    Self::inject_hoisted_qrls_into_body(&ast_builder, &mut body.statements, &hoisted_qrls);
                }
            }
            Expression::ArrowFunctionExpression(arrow) => {
                if !arrow.expression {
                    Self::inject_hoisted_qrls_into_body(&ast_builder, &mut arrow.body.statements, &hoisted_qrls);
                }
            }
            _ => {}
        }

        let bind_pat = ast_builder.binding_pattern_binding_identifier(SPAN, name.as_str());
        let mut var_declarator = OxcVec::new_in(allocator);

        var_declarator.push(ast_builder.variable_declarator(
            SPAN,
            VariableDeclarationKind::Const,
            bind_pat,
            None::<OxcBox<'_, TSTypeAnnotation<'_>>>,
            Some(transformed_expression),
            false,
        ));

        let decl = ast_builder.variable_declaration(
            SPAN,
            VariableDeclarationKind::Const,
            var_declarator,
            false,
        );
        let decl = OxcBox::new_in(decl, allocator);
        let decl = Declaration::VariableDeclaration(decl);
        let export = ast_builder.export_named_declaration(
            SPAN,
            Some(decl),
            OxcVec::new_in(allocator),
            None,
            ImportOrExportKind::Value,
            None::<OxcBox<WithClause>>,
        );
        let export = Statement::ExportNamedDeclaration(OxcBox::new_in(export, allocator));

        let imports_statements = imports.iter().map(|import| {
            let statement: Statement = import.clone().into_in(allocator);
            statement
        });

        let mut body = ast_builder.vec_from_iter(imports_statements);

        // Generate hoisted import declarations for child QRLs (module level)
        for (ident_name, filename) in hoisted_imports.iter().rev() {
            let hoisted_stmt = Statement::VariableDeclaration(ast_builder.alloc(ast_builder.variable_declaration(
                SPAN,
                VariableDeclarationKind::Const,
                ast_builder.vec1(ast_builder.variable_declarator(
                    SPAN,
                    VariableDeclarationKind::Const,
                    ast_builder.binding_pattern_binding_identifier(SPAN, ast_builder.atom(ident_name)),
                    None::<OxcBox<'_, TSTypeAnnotation<'_>>>,
                    Some(ast_builder.expression_arrow_function(
                        SPAN,
                        true,
                        false,
                        None::<OxcBox<TSTypeParameterDeclaration>>,
                        ast_builder.formal_parameters(
                            SPAN,
                            FormalParameterKind::ArrowFormalParameters,
                            ast_builder.vec(),
                            NONE,
                        ),
                        None::<OxcBox<TSTypeAnnotation>>,
                        ast_builder.function_body(
                            SPAN,
                            ast_builder.vec(),
                            ast_builder.vec1(ast_builder.statement_expression(
                                SPAN,
                                ast_builder.expression_import(
                                    SPAN,
                                    ast_builder.expression_string_literal(SPAN, ast_builder.atom(filename), None),
                                    None,
                                    None,
                                ),
                            )),
                        ),
                    )),
                    false,
                )),
                false,
            )));
            body.push(hoisted_stmt);
        }

        // Generate hoisted function declarations for _fnSignal (module level)
        for (hf_name, hf_code, hf_str) in hoisted_fns.iter() {
            let hf_allocator = Allocator::default();
            let parsed = oxc_parser::Parser::new(&hf_allocator, hf_code, SourceType::default())
                .parse_expression();
            if let Ok(hf_expr) = parsed {
                let hf_expr_cloned = hf_expr.clone_in(allocator);

                let hf_stmt = Statement::VariableDeclaration(ast_builder.alloc(ast_builder.variable_declaration(
                    SPAN,
                    VariableDeclarationKind::Const,
                    ast_builder.vec1(ast_builder.variable_declarator(
                        SPAN,
                        VariableDeclarationKind::Const,
                        ast_builder.binding_pattern_binding_identifier(SPAN, ast_builder.atom(hf_name)),
                        None::<OxcBox<'_, TSTypeAnnotation<'_>>>,
                        Some(hf_expr_cloned),
                        false,
                    )),
                    false,
                )));
                body.push(hf_stmt);

                let hf_str_name = format!("{}_str", hf_name);
                let hf_str_stmt = Statement::VariableDeclaration(ast_builder.alloc(ast_builder.variable_declaration(
                    SPAN,
                    VariableDeclarationKind::Const,
                    ast_builder.vec1(ast_builder.variable_declarator(
                        SPAN,
                        VariableDeclarationKind::Const,
                        ast_builder.binding_pattern_binding_identifier(SPAN, ast_builder.atom(&hf_str_name)),
                        None::<OxcBox<'_, TSTypeAnnotation<'_>>>,
                        Some(ast_builder.expression_string_literal(SPAN, ast_builder.atom(hf_str), None)),
                        false,
                    )),
                    false,
                )));
                body.push(hf_str_stmt);
            }
        }

        body.push(export);

        let ast_builder = AstBuilder::new(allocator);

        let mut new_pgm = ast_builder.program(
            SPAN,
            *source_type,
            "",
            OxcVec::new_in(allocator),
            None,
            OxcVec::new_in(allocator),
            body,
        );

        ImportCleanUp::clean_up(&mut new_pgm, allocator);

        let should_minify = if options.format_output {
            false
        } else {
            options.minify
        };

        let codegen = Codegen::new();
        let codegen_options = CodegenOptions {
            minify: should_minify,
            ..Default::default()
        };

        let code = if options.minify && !options.format_output {
            let ops = MinifierOptions {
                compress: Some(CompressOptions::default()),
                mangle: None,
            };
            let minifier = Minifier::new(ops);
            let ret = minifier.minify(allocator, &mut new_pgm);
            let scoping = ret.scoping;

            codegen
                .with_options(codegen_options)
                .with_scoping(scoping)
                .build(&new_pgm)
                .code
        } else {
            codegen.with_options(codegen_options).build(&new_pgm).code
        };
        let code = code.replace("/* @__PURE__ */", "/*#__PURE__*/");
        code.replace(") => ", ")=>")
    }

    /// Inject hoisted QRL const declarations into a function body.
    /// Format: const Name = /*#__PURE__*/ qrl(i_xxx, "symbol_name");
    /// Declarations are inserted AFTER existing const declarations but BEFORE the return statement.
    fn inject_hoisted_qrls_into_body<'a>(
        ast_builder: &AstBuilder<'a>,
        statements: &mut OxcVec<'a, Statement<'a>>,
        hoisted_qrls: &[(String, String)],
    ) {
        // Find the position to insert: after const declarations, before return
        let insert_pos = Self::find_insert_position(statements);

        // Parse each QRL declaration and create a const statement
        // Insert in reverse order so they appear in correct order after all insertions
        for (const_name, qrl_call_code) in hoisted_qrls.iter().rev() {
            let qrl_allocator = Allocator::default();
            let parsed = oxc_parser::Parser::new(&qrl_allocator, qrl_call_code, SourceType::default())
                .parse_expression();

            if let Ok(qrl_expr) = parsed {
                let qrl_expr_cloned = qrl_expr.clone_in(ast_builder.allocator);

                // Create: const ConstName = /*#__PURE__*/ qrl(...);
                let const_stmt = Statement::VariableDeclaration(ast_builder.alloc(ast_builder.variable_declaration(
                    SPAN,
                    VariableDeclarationKind::Const,
                    ast_builder.vec1(ast_builder.variable_declarator(
                        SPAN,
                        VariableDeclarationKind::Const,
                        ast_builder.binding_pattern_binding_identifier(SPAN, ast_builder.atom(const_name)),
                        None::<OxcBox<'_, TSTypeAnnotation<'_>>>,
                        Some(qrl_expr_cloned),
                        false,
                    )),
                    false,
                )));

                statements.insert(insert_pos, const_stmt);
            }
        }
    }

    /// Find the position to insert hoisted QRL declarations.
    /// Insert after const declarations but before the return statement.
    fn find_insert_position(statements: &[Statement]) -> usize {
        // Find the first return statement
        for (i, stmt) in statements.iter().enumerate() {
            if matches!(stmt, Statement::ReturnStatement(_)) {
                return i;
            }
        }
        // If no return found, insert at end
        statements.len()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_expression(
        expr: Expression<'_>,
        imports: Vec<Import>,
        segments: &Vec<Segment>,
        scope: &Option<String>,
        options: &TransformOptions,
        source_info: &SourceInfo,
        segment_data: Option<SegmentData>,
        entry: Option<String>,
    ) -> QrlComponent {
        let qrl_type: QrlType = segments
            .last()
            .iter()
            .flat_map(|segment| segment.qrl_type())
            .last()
            .unwrap(); // TODO Clean this up.

        let id = Id::new(source_info, segments, &options.target, scope);

        QrlComponent::new(options, source_info, id, expr, imports, qrl_type, segment_data, entry)
    }

    /// Creates a QrlComponent with explicit QrlType (for event handlers in loops).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_expression_with_qrl_type(
        expr: Expression<'_>,
        imports: Vec<Import>,
        segments: &Vec<Segment>,
        scope: &Option<String>,
        options: &TransformOptions,
        source_info: &SourceInfo,
        segment_data: Option<SegmentData>,
        entry: Option<String>,
        qrl_type: QrlType,
    ) -> QrlComponent {
        let id = Id::new(source_info, segments, &options.target, scope);
        QrlComponent::new(options, source_info, id, expr, imports, qrl_type, segment_data, entry)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_call_expression_argument(
        arg: &Argument,
        imports: Vec<Import>,
        hoisted_imports: Vec<(String, String)>,
        segments: &Vec<Segment>,
        scope: &Option<String>,
        options: &TransformOptions,
        source_info: &SourceInfo,
        segment_data: Option<SegmentData>,
        entry: Option<String>,
        allocator: &Allocator,
    ) -> QrlComponent {
        let init = arg.clone_in(allocator).into_expression();
        Self::from_expression_with_hoisted_imports(init, imports, hoisted_imports, segments, scope, options, source_info, segment_data, entry)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_expression_with_hoisted_imports(
        expr: Expression<'_>,
        imports: Vec<Import>,
        hoisted_imports: Vec<(String, String)>,
        segments: &Vec<Segment>,
        scope: &Option<String>,
        options: &TransformOptions,
        source_info: &SourceInfo,
        segment_data: Option<SegmentData>,
        entry: Option<String>,
    ) -> QrlComponent {
        let qrl_type: QrlType = segments
            .last()
            .iter()
            .flat_map(|segment| segment.qrl_type())
            .last()
            .unwrap();

        let id = Id::new(source_info, segments, &options.target, scope);

        QrlComponent::new_with_hoisted_imports(options, source_info, id, expr, imports, hoisted_imports, qrl_type, segment_data, entry)
    }

    /// Creates a QrlComponent from a call expression argument with hoisted functions.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_call_expression_argument_with_hoisted_fns(
        arg: &Argument,
        imports: Vec<Import>,
        hoisted_imports: Vec<(String, String)>,
        hoisted_fns: Vec<(String, String, String)>,
        segments: &Vec<Segment>,
        scope: &Option<String>,
        options: &TransformOptions,
        source_info: &SourceInfo,
        segment_data: Option<SegmentData>,
        entry: Option<String>,
        allocator: &Allocator,
    ) -> QrlComponent {
        let init = arg.clone_in(allocator).into_expression();
        Self::from_expression_with_hoisted_fns(init, imports, hoisted_imports, hoisted_fns, segments, scope, options, source_info, segment_data, entry)
    }

    /// Creates a QrlComponent from a call expression argument with hoisted functions and QRLs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_call_expression_argument_with_hoisted_all(
        arg: &Argument,
        imports: Vec<Import>,
        hoisted_imports: Vec<(String, String)>,
        hoisted_fns: Vec<(String, String, String)>,
        hoisted_qrls: Vec<(String, String)>,
        segments: &Vec<Segment>,
        scope: &Option<String>,
        options: &TransformOptions,
        source_info: &SourceInfo,
        segment_data: Option<SegmentData>,
        entry: Option<String>,
        allocator: &Allocator,
    ) -> QrlComponent {
        let init = arg.clone_in(allocator).into_expression();
        Self::from_expression_with_hoisted_all(init, imports, hoisted_imports, hoisted_fns, hoisted_qrls, segments, scope, options, source_info, segment_data, entry)
    }

    /// Creates a QrlComponent from an expression with hoisted functions.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_expression_with_hoisted_fns(
        expr: Expression<'_>,
        imports: Vec<Import>,
        hoisted_imports: Vec<(String, String)>,
        hoisted_fns: Vec<(String, String, String)>,
        segments: &Vec<Segment>,
        scope: &Option<String>,
        options: &TransformOptions,
        source_info: &SourceInfo,
        segment_data: Option<SegmentData>,
        entry: Option<String>,
    ) -> QrlComponent {
        let qrl_type: QrlType = segments
            .last()
            .iter()
            .flat_map(|segment| segment.qrl_type())
            .last()
            .unwrap();

        let id = Id::new(source_info, segments, &options.target, scope);

        QrlComponent::new_with_hoisted(options, source_info, id, expr, imports, hoisted_imports, hoisted_fns, qrl_type, segment_data, entry)
    }

    /// Creates a QrlComponent from an expression with hoisted functions and QRLs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_expression_with_hoisted_all(
        expr: Expression<'_>,
        imports: Vec<Import>,
        hoisted_imports: Vec<(String, String)>,
        hoisted_fns: Vec<(String, String, String)>,
        hoisted_qrls: Vec<(String, String)>,
        segments: &Vec<Segment>,
        scope: &Option<String>,
        options: &TransformOptions,
        source_info: &SourceInfo,
        segment_data: Option<SegmentData>,
        entry: Option<String>,
    ) -> QrlComponent {
        let qrl_type: QrlType = segments
            .last()
            .iter()
            .flat_map(|segment| segment.qrl_type())
            .last()
            .unwrap();

        let id = Id::new(source_info, segments, &options.target, scope);

        QrlComponent::new_with_hoisted_all(options, source_info, id, expr, imports, hoisted_imports, hoisted_fns, hoisted_qrls, qrl_type, segment_data, entry)
    }

    pub fn has_captures(&self) -> bool {
        self.segment_data
            .as_ref()
            .map(|d| d.has_captures())
            .unwrap_or(false)
    }
}
