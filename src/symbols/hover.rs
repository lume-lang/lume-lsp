use lume_errors::Result;
use lume_hir::Identifier;
use lume_infer::query::CallReference;
use lume_span::{Location, NodeId};

use crate::state::State;
use crate::symbols::lookup::SymbolKind;

pub(crate) fn hover_content_of(state: &State, location: Location) -> Result<String> {
    let Some(sym) = state.checked.symbols.lookup_position(location) else {
        log::warn!("could not find matching node for {location}");
        return Ok(String::new());
    };

    match &sym.kind {
        SymbolKind::Type { name } => hover_content_of_type(state, location, name),
        SymbolKind::Callable { reference } => hover_content_of_callable(state, location, *reference),
        SymbolKind::Member { callee, field } => hover_content_of_member(state, location, *callee, field),
        SymbolKind::Variant { name } => hover_content_of_variant(state, location, name),
        SymbolKind::Pattern { id } => hover_content_of_pattern(state, location, *id),
    }
}

pub(crate) fn hover_content_of_type(state: &State, location: Location, type_name: &lume_hir::Path) -> Result<String> {
    let package = state.checked.graph.packages.get(&location.file.package).unwrap();
    let Some(type_id) = package.tcx.tdb().find_type(type_name).map(|ty| ty.id) else {
        return Ok(String::new());
    };

    let Some(lume_hir::Node::Type(type_def)) = package.tcx.hir_node(type_id) else {
        return Ok(String::new());
    };

    match type_def {
        lume_hir::TypeDefinition::Struct(struct_def) => {
            let builtin = if struct_def.builtin {
                String::from("builtin ")
            } else {
                String::new()
            };

            Ok(format!(
                "```lm\n{} struct {builtin}{:+}\n```",
                struct_def.visibility, struct_def.name
            ))
        }
        lume_hir::TypeDefinition::Trait(trait_def) => Ok(format!(
            "```lm\n{} trait {:+}\n```",
            trait_def.visibility, trait_def.name
        )),
        lume_hir::TypeDefinition::Enum(enum_def) => {
            Ok(format!("```lm\n{} enum {:+}\n```", enum_def.visibility, enum_def.name))
        }
    }
}

pub(crate) fn hover_content_of_callable(state: &State, location: Location, reference: CallReference) -> Result<String> {
    let package = state.checked.graph.packages.get(&location.file.package).unwrap();
    let callable = package.tcx.callable_of(reference)?;

    let identifier = lume_hir::Identifier {
        name: format!("{:+}", callable.name()),
        location: callable.name().location,
    };

    let signature = package.tcx.sig_to_string(&identifier, callable.signature(), false)?;
    let visibility = match package.tcx.visibility_of(callable.id()) {
        Some(visibility) => format!("{visibility} "),
        None => String::new(),
    };

    Ok(format!("```lm\n{visibility}{signature}\n```"))
}

pub(crate) fn hover_content_of_member(
    state: &State,
    location: Location,
    callee: NodeId,
    field: &Identifier,
) -> Result<String> {
    let package = state.checked.graph.packages.get(&location.file.package).unwrap();

    let callee_type = package.tcx.type_of(callee)?;
    let Some(field) = package.tcx.tdb().find_field(callee_type.instance_of, &field.name) else {
        return Ok(String::new());
    };

    let field_type = package.tcx.new_named_type(&field.field_type, true)?;

    Ok(format!(
        "```lm\n{} {}: {field_type};\n```",
        field.visibility, field.name
    ))
}

pub(crate) fn hover_content_of_variant(state: &State, location: Location, name: &lume_hir::Path) -> Result<String> {
    let package = state.checked.graph.packages.get(&location.file.package).unwrap();

    let enum_name = name.clone().parent().unwrap();
    let enum_def = package.tcx.enum_def_of_name(&enum_name)?;
    let enum_case = package.tcx.enum_case_with_name(name)?;

    let fields = if enum_case.parameters.is_empty() {
        String::new()
    } else {
        let fields = enum_case
            .parameters
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<String>>()
            .join(", ");

        format!("({fields})")
    };

    Ok(format!("```lm\n{:+}::{}{fields}\n```", enum_def.name, enum_case.name))
}

pub(crate) fn hover_content_of_pattern(state: &State, location: Location, id: NodeId) -> Result<String> {
    let package = state.checked.graph.packages.get(&location.file.package).unwrap();

    let Some(lume_hir::Node::Pattern(pattern)) = package.tcx.hir_node(id) else {
        return Ok(String::new());
    };

    let pattern_ty = package.tcx.type_of_pattern(pattern)?;
    let pattern_ty_name = package.tcx.new_named_type(&pattern_ty, true)?;

    Ok(format!("```lm\n{pattern_ty_name}\n```"))
}
