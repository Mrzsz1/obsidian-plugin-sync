use crate::{
    errors::{AppError, AppResult},
    fs_safety,
    models::{
        ManagedPluginSettings, PluginRuntimeSettingField, PluginRuntimeSettingsSnapshot,
        PluginSettingConfidence, PluginSettingControl, PluginSettingField, PluginSettingGroup,
        PluginSettingOption, PluginSettingPathOption, PluginSettingSource, PluginSettingSupport,
        PluginSettingsCompleteness, PluginSettingsCoverage, PluginSettingsSchema,
        PluginSettingsSchemaSource,
    },
    plugin_adapters::inspect_plugin_adapter,
    plugin_manager::scan_plugin_management_inventory,
    settings_bridge::inspect_bridge_for_plugin,
};
use serde_json::{Map, Number, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};
use tree_sitter::{Node, Parser};

const MAX_MAIN_JS_BYTES: u64 = 16 * 1024 * 1024;
const I18N_CONSTANT_PREFIX: &str = "__obsidian_plugin_sync_i18n__:";
const UNRESOLVED_I18N_PREFIX: &str = "__obsidian_plugin_sync_unresolved_i18n__:";

#[derive(Debug)]
struct ExtractedField {
    field: PluginSettingField,
    page_path: Vec<String>,
}

#[derive(Debug, Default)]
struct ExtractionResult {
    fields: Vec<ExtractedField>,
    warnings: Vec<String>,
    found_declarative: bool,
    found_imperative: bool,
}

#[derive(Clone, Copy)]
struct RegisteredSettingTab<'tree> {
    class_node: Node<'tree>,
}

#[derive(Clone)]
struct ReachableScope<'tree> {
    node: Node<'tree>,
    containers: BTreeMap<String, Vec<String>>,
}

#[derive(Clone)]
struct RegisteredRenderer<'tree> {
    render_node: Node<'tree>,
    page_name: String,
}

#[derive(Default)]
struct SettingsProgramIndex<'tree> {
    aliases: ObsidianAliases,
    functions: BTreeMap<String, Node<'tree>>,
    values: BTreeMap<String, Node<'tree>>,
    renderers: Vec<RegisteredRenderer<'tree>>,
}

#[derive(Default)]
struct ObsidianAliases {
    plugin_setting_tabs: BTreeSet<String>,
    settings: BTreeSet<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConditionalVisibility {
    Visible,
    Hidden,
    Unknown,
}

struct ReachableSetting<'tree> {
    statement: Node<'tree>,
    page_path: Vec<String>,
    condition_unknown: bool,
}

pub fn inspect_plugin_settings(
    vault_path: String,
    plugin_id: String,
) -> AppResult<ManagedPluginSettings> {
    let inventory = scan_plugin_management_inventory(vault_path)?;
    let vault_root = PathBuf::from(&inventory.vault.path);
    let managed = inventory
        .plugins
        .into_iter()
        .find(|item| item.plugin.id.as_deref() == Some(plugin_id.as_str()))
        .ok_or_else(|| AppError::new("missing_plugin", "知识库中不存在该插件"))?;

    if !managed.plugin.valid || managed.plugin.unsupported_reason.is_some() {
        return Err(AppError::new(
            "unsupported_plugin",
            "该插件目录不受支持，无法推断设置",
        ));
    }

    let plugin_dir = PathBuf::from(&managed.plugin.folder_path);
    fs_safety::ensure_child_path(&vault_root, &plugin_dir)?;
    if fs_safety::is_link_path(&plugin_dir)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持链接目录插件").with_path(&plugin_dir)
        );
    }

    let main_path = plugin_dir.join("main.js");
    fs_safety::ensure_child_path(&vault_root, &main_path)?;
    let mut initial_warnings = Vec::new();
    if let Some(error) = managed.configuration_error.as_deref() {
        initial_warnings.push(error.to_string());
    }

    let source = read_bounded_source(&main_path, &mut initial_warnings)?;
    let mut schema = infer_settings_schema(
        source.as_deref(),
        managed.configuration.as_ref(),
        initial_warnings,
    );
    let bridge_inspection = inspect_bridge_for_plugin(&vault_root, &managed.plugin);
    let runtime_snapshot: Option<PluginRuntimeSettingsSnapshot> = bridge_inspection.snapshot;
    if let Some(snapshot) = runtime_snapshot.as_ref() {
        merge_runtime_settings_presentation(&mut schema, snapshot);
    }
    let adapter = inspect_plugin_adapter(&managed.plugin, managed.configuration.as_ref());

    Ok(ManagedPluginSettings {
        plugin_id,
        configuration: managed.configuration,
        configuration_error: managed.configuration_error,
        schema,
        runtime_snapshot,
        bridge: bridge_inspection.status,
        adapter,
    })
}

fn read_bounded_source(path: &Path, warnings: &mut Vec<String>) -> AppResult<Option<String>> {
    if !path.exists() {
        warnings.push("插件缺少 main.js，无法安全识别可见设置".to_string());
        return Ok(None);
    }
    if fs_safety::is_link_path(path)? {
        return Err(
            AppError::new("unsupported_link_path", "不支持读取链接形式的 main.js").with_path(path),
        );
    }
    let metadata = fs::metadata(path).map_err(|error| AppError::from(error).with_path(path))?;
    if metadata.len() > MAX_MAIN_JS_BYTES {
        warnings.push(format!(
            "main.js 大于 {} MiB，已跳过设置分析",
            MAX_MAIN_JS_BYTES / 1024 / 1024
        ));
        return Ok(None);
    }
    match fs::read_to_string(path) {
        Ok(source) => Ok(Some(source)),
        Err(error) => {
            warnings.push(format!("无法读取 main.js，无法安全识别可见设置：{error}"));
            Ok(None)
        }
    }
}

fn infer_settings_schema(
    source: Option<&str>,
    configuration: Option<&Value>,
    mut warnings: Vec<String>,
) -> PluginSettingsSchema {
    let mut extraction = source
        .map(|source| extract_from_javascript(source, configuration))
        .unwrap_or_default();
    warnings.append(&mut extraction.warnings);

    let mut seen_paths = BTreeSet::new();
    let mut deduplicated = Vec::new();
    for item in extraction.fields {
        let should_keep = match item.field.path.as_ref() {
            Some(path) => seen_paths.insert(path.clone()),
            None => true,
        };
        if should_keep {
            deduplicated.push(item);
        }
    }

    for item in &mut deduplicated {
        let action_only = matches!(item.field.support, PluginSettingSupport::ActionOnly);
        item.field.support = classify_field_support(&item.field, action_only);
    }
    let mut groups = build_groups(deduplicated);

    if groups.is_empty() {
        groups.push(PluginSettingGroup {
            id: "empty-settings".to_string(),
            title: None,
            page_path: Vec::new(),
            fields: Vec::new(),
        });
    }

    let source_kind = match (extraction.found_declarative, extraction.found_imperative) {
        (true, true) => PluginSettingsSchemaSource::Mixed,
        (true, false) => PluginSettingsSchemaSource::Declarative,
        (false, true) => PluginSettingsSchemaSource::Imperative,
        (false, false) => PluginSettingsSchemaSource::DataJson,
    };
    let completeness = if matches!(source_kind, PluginSettingsSchemaSource::DataJson) {
        PluginSettingsCompleteness::Fallback
    } else if !warnings.is_empty() {
        PluginSettingsCompleteness::Partial
    } else {
        PluginSettingsCompleteness::Complete
    };

    if matches!(source_kind, PluginSettingsSchemaSource::DataJson)
        && !warnings.iter().any(|warning| warning.contains("main.js"))
    {
        warnings.push(
            "未识别到可安全映射的 Obsidian 设置；未展示 data.json 中未证实的字段".to_string(),
        );
    }

    PluginSettingsSchema {
        source: source_kind,
        completeness,
        coverage: settings_coverage(&groups),
        groups,
        warnings,
    }
}

fn extract_from_javascript(source: &str, configuration: Option<&Value>) -> ExtractionResult {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .is_err()
    {
        return ExtractionResult {
            warnings: vec!["无法初始化 JavaScript 解析器，无法安全识别可见设置".to_string()],
            ..ExtractionResult::default()
        };
    }
    let Some(tree) = parser.parse(source, None) else {
        return ExtractionResult {
            warnings: vec!["无法解析 main.js，无法安全识别可见设置".to_string()],
            ..ExtractionResult::default()
        };
    };
    let root = tree.root_node();
    let mut constants = collect_static_constants(root, source);
    collect_translation_constants(root, source, configuration, &mut constants);
    let program_index = build_program_index(root, source);
    let mut result = ExtractionResult::default();
    if root.has_error() {
        result
            .warnings
            .push("main.js 包含无法完整解析的语法，设置结果可能不完整".to_string());
    }

    let setting_tabs = find_registered_setting_tabs(root, source, &program_index.aliases);
    if setting_tabs.is_empty() {
        result
            .warnings
            .push("未找到可静态确认的 addSettingTab 注册，已停止设置提取".to_string());
        return result;
    }

    let declarative =
        extract_declarative_fields(&setting_tabs, source, &constants, &mut result.warnings);
    result.found_declarative = !declarative.is_empty();
    result.fields.extend(declarative);
    if !result.found_declarative {
        let imperative = extract_imperative_fields(
            &setting_tabs,
            source,
            &constants,
            configuration,
            &program_index,
            &mut result.warnings,
        );
        result.found_imperative = !imperative.is_empty();
        result.fields.extend(imperative);
    }
    result
}

fn find_registered_setting_tabs<'tree>(
    root: Node<'tree>,
    source: &str,
    aliases: &ObsidianAliases,
) -> Vec<RegisteredSettingTab<'tree>> {
    let mut classes = BTreeMap::<String, Node<'tree>>::new();
    let mut new_bindings = BTreeMap::<String, String>::new();
    collect_nodes(root, &mut |node| match node.kind() {
        "class" | "class_declaration"
            if class_extends_plugin_setting_tab(node, source, aliases) =>
        {
            if let Some(name) = class_symbol_name(node, source) {
                classes.insert(name, node);
            }
        }
        "variable_declarator" => {
            let Some(name) = node.child_by_field_name("name") else {
                return;
            };
            let Some(value) = node.child_by_field_name("value") else {
                return;
            };
            if name.kind() == "identifier" && value.kind() == "new_expression" {
                if let Some(constructor) = new_expression_constructor_name(value, source) {
                    new_bindings.insert(node_text(name, source).to_string(), constructor);
                }
            }
        }
        _ => {}
    });

    let mut registered_names = BTreeSet::new();
    collect_nodes(root, &mut |node| {
        if node.kind() != "call_expression"
            || call_method_name(node, source).as_deref() != Some("addSettingTab")
        {
            return;
        }
        let Some(argument) = call_arguments(node).first().copied() else {
            return;
        };
        let name = if argument.kind() == "new_expression" {
            new_expression_constructor_name(argument, source)
        } else if argument.kind() == "identifier" {
            new_bindings.get(node_text(argument, source)).cloned()
        } else {
            None
        };
        if let Some(name) = name {
            registered_names.insert(name);
        }
    });

    registered_names
        .into_iter()
        .filter_map(|name| classes.get(&name).copied())
        .map(|class_node| RegisteredSettingTab { class_node })
        .collect()
}

fn class_extends_plugin_setting_tab(
    node: Node<'_>,
    source: &str,
    aliases: &ObsidianAliases,
) -> bool {
    let mut cursor = node.walk();
    let found = node.named_children(&mut cursor).any(|child| {
        if child.kind() != "class_heritage" {
            return false;
        }
        let heritage = node_text(child, source)
            .trim_start_matches("extends")
            .trim();
        let base = heritage.rsplit('.').next().unwrap_or(heritage);
        aliases.plugin_setting_tabs.contains(base)
    });
    found
}

fn class_symbol_name(node: Node<'_>, source: &str) -> Option<String> {
    if let Some(name) = node.child_by_field_name("name") {
        return Some(node_text(name, source).to_string());
    }
    let parent = node.parent()?;
    if parent.kind() == "variable_declarator" && parent.child_by_field_name("value") == Some(node) {
        let name = parent.child_by_field_name("name")?;
        if name.kind() == "identifier" {
            return Some(node_text(name, source).to_string());
        }
    }
    None
}

fn new_expression_constructor_name(node: Node<'_>, source: &str) -> Option<String> {
    let constructor = node.child_by_field_name("constructor")?;
    node_text(constructor, source)
        .rsplit('.')
        .next()
        .map(str::to_string)
}

fn class_methods<'tree>(class_node: Node<'tree>, source: &str) -> BTreeMap<String, Node<'tree>> {
    let mut methods = BTreeMap::new();
    collect_nodes(class_node, &mut |node| {
        if node.kind() != "method_definition" || !belongs_to_class(node, class_node) {
            return;
        }
        if let Some(name) = declared_node_name(node, source) {
            methods.insert(name, node);
        }
    });
    methods
}

fn belongs_to_class(mut node: Node<'_>, class_node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        if matches!(parent.kind(), "class" | "class_declaration") {
            return parent == class_node;
        }
        node = parent;
    }
    false
}

fn declared_node_name(node: Node<'_>, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|name| node_text(name, source).to_string())
        .or_else(|| {
            node.child_by_field_name("key")
                .and_then(|key| property_key(key, source))
        })
}

fn collect_obsidian_aliases(root: Node<'_>, source: &str) -> ObsidianAliases {
    let mut aliases = ObsidianAliases::default();
    aliases
        .plugin_setting_tabs
        .insert("PluginSettingTab".to_string());
    aliases.settings.insert("Setting".to_string());
    let mut namespaces = BTreeSet::new();

    collect_nodes(root, &mut |node| match node.kind() {
        "variable_declarator" => {
            let Some(name) = node.child_by_field_name("name") else {
                return;
            };
            let Some(value) = node.child_by_field_name("value") else {
                return;
            };
            if !is_obsidian_require(value, source) {
                return;
            }
            if name.kind() == "identifier" {
                namespaces.insert(node_text(name, source).to_string());
            } else if name.kind() == "object_pattern" {
                collect_obsidian_object_pattern_aliases(name, source, &mut aliases);
            }
        }
        "import_statement" if import_is_from_obsidian(node, source) => {
            let text = node_text(node, source);
            if let Some(namespace) = import_namespace_alias(text) {
                namespaces.insert(namespace);
            }
            collect_nodes(node, &mut |specifier| {
                if specifier.kind() != "import_specifier" {
                    return;
                }
                let parts = node_text(specifier, source)
                    .split_whitespace()
                    .collect::<Vec<_>>();
                let Some(imported) = parts.first().copied() else {
                    return;
                };
                let local = if parts.get(1) == Some(&"as") {
                    parts.get(2).copied().unwrap_or(imported)
                } else {
                    imported
                };
                insert_obsidian_alias(imported, local, &mut aliases);
            });
        }
        _ => {}
    });

    collect_nodes(root, &mut |node| {
        if node.kind() != "variable_declarator" {
            return;
        }
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let Some(value) = node.child_by_field_name("value") else {
            return;
        };
        if name.kind() != "identifier" || value.kind() != "member_expression" {
            return;
        }
        let Some(object) = value.child_by_field_name("object") else {
            return;
        };
        let Some(property) = value.child_by_field_name("property") else {
            return;
        };
        if object.kind() != "identifier" || !namespaces.contains(node_text(object, source)) {
            return;
        }
        insert_obsidian_alias(
            node_text(property, source),
            node_text(name, source),
            &mut aliases,
        );
    });
    aliases
}

fn is_obsidian_require(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "call_expression"
        || call_method_name(node, source).as_deref() != Some("require")
    {
        return false;
    }
    call_arguments(node)
        .first()
        .and_then(|argument| parse_js_string(node_text(*argument, source)))
        .as_deref()
        == Some("obsidian")
}

fn import_is_from_obsidian(node: Node<'_>, source: &str) -> bool {
    let text = node_text(node, source);
    text.contains("'obsidian'") || text.contains("\"obsidian\"")
}

fn import_namespace_alias(statement: &str) -> Option<String> {
    let marker = statement.find("* as ")? + "* as ".len();
    let alias = statement[marker..]
        .split(|character: char| character.is_whitespace() || character == ';')
        .next()?;
    (!alias.is_empty()).then(|| alias.to_string())
}

fn collect_obsidian_object_pattern_aliases(
    pattern: Node<'_>,
    source: &str,
    aliases: &mut ObsidianAliases,
) {
    let mut cursor = pattern.walk();
    for child in pattern.named_children(&mut cursor) {
        if let Some((imported, local)) = object_pattern_entry(child, source) {
            insert_obsidian_alias(&imported, &local, aliases);
        }
    }
}

fn insert_obsidian_alias(imported: &str, local: &str, aliases: &mut ObsidianAliases) {
    match imported {
        "PluginSettingTab" => {
            aliases.plugin_setting_tabs.insert(local.to_string());
        }
        "Setting" => {
            aliases.settings.insert(local.to_string());
        }
        _ => {}
    }
}

fn build_program_index<'tree>(root: Node<'tree>, source: &str) -> SettingsProgramIndex<'tree> {
    let aliases = collect_obsidian_aliases(root, source);
    let mut function_candidates = BTreeMap::<String, Vec<Node<'tree>>>::new();
    let mut value_candidates = BTreeMap::<String, Vec<Node<'tree>>>::new();
    let mut object_bindings = BTreeMap::<String, Node<'tree>>::new();
    let mut renderer_bindings = BTreeSet::<String>::new();

    collect_nodes(root, &mut |node| match node.kind() {
        "function_declaration" => {
            if let Some(name) = declared_node_name(node, source) {
                function_candidates.entry(name).or_default().push(node);
            }
        }
        "variable_declarator" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                return;
            };
            let Some(value_node) = node.child_by_field_name("value") else {
                return;
            };
            if name_node.kind() != "identifier" {
                return;
            }
            let name = node_text(name_node, source).to_string();
            value_candidates
                .entry(name.clone())
                .or_default()
                .push(value_node);
            if matches!(value_node.kind(), "arrow_function" | "function_expression") {
                function_candidates
                    .entry(name)
                    .or_default()
                    .push(value_node);
            } else if value_node.kind() == "object" {
                object_bindings.insert(name, value_node);
            }
        }
        "pair" => {
            let Some(key) = node.child_by_field_name("key") else {
                return;
            };
            if property_key(key, source).as_deref() != Some("settingsTabRenderer") {
                return;
            }
            let Some(value) = node.child_by_field_name("value") else {
                return;
            };
            if value.kind() == "identifier" {
                renderer_bindings.insert(node_text(value, source).to_string());
            }
        }
        _ => {}
    });

    let functions = function_candidates
        .into_iter()
        .filter_map(|(name, nodes)| (nodes.len() == 1).then(|| (name, nodes[0])))
        .collect();
    let values = value_candidates
        .into_iter()
        .filter_map(|(name, nodes)| (nodes.len() == 1).then(|| (name, nodes[0])))
        .collect();
    let renderers = renderer_bindings
        .into_iter()
        .filter_map(|binding| {
            let object = object_bindings.get(&binding).copied()?;
            let render_node = object_callable(object, "render", source)?;
            Some(RegisteredRenderer {
                render_node,
                page_name: renderer_page_name(&binding),
            })
        })
        .collect();

    SettingsProgramIndex {
        aliases,
        functions,
        values,
        renderers,
    }
}

fn object_callable<'tree>(
    object: Node<'tree>,
    expected: &str,
    source: &str,
) -> Option<Node<'tree>> {
    let mut cursor = object.walk();
    let callable = object.named_children(&mut cursor).find_map(|child| {
        if child.kind() == "method_definition"
            && declared_node_name(child, source).as_deref() == Some(expected)
        {
            return Some(child);
        }
        if child.kind() != "pair" {
            return None;
        }
        let key = child.child_by_field_name("key")?;
        if property_key(key, source).as_deref() != Some(expected) {
            return None;
        }
        let value = child.child_by_field_name("value")?;
        matches!(value.kind(), "arrow_function" | "function_expression").then_some(value)
    });
    callable
}

fn renderer_page_name(binding: &str) -> String {
    let suffix = "SettingsTabRenderer";
    let base = binding.strip_suffix(suffix).unwrap_or(binding);
    if base.is_empty() {
        "provider".to_string()
    } else {
        base.to_ascii_lowercase()
    }
}

fn extract_declarative_fields<'tree>(
    setting_tabs: &[RegisteredSettingTab<'tree>],
    source: &str,
    constants: &HashMap<String, Value>,
    warnings: &mut Vec<String>,
) -> Vec<ExtractedField> {
    let mut methods = Vec::new();
    for tab in setting_tabs {
        if let Some(method) = class_methods(tab.class_node, source).get("getSettingDefinitions") {
            methods.push(*method);
        }
    }

    let mut output = Vec::new();
    for method in methods {
        let mut returns = Vec::new();
        collect_nodes(method, &mut |node| {
            if node.kind() == "return_statement" {
                returns.push(node);
            }
        });
        for return_node in returns {
            let Some(expression) = return_node.named_child(0) else {
                continue;
            };
            if expression.kind() == "array" {
                parse_declarative_array(expression, source, constants, warnings, &mut output);
            } else if let Some(Value::Array(items)) = static_value(expression, source, constants, 0)
            {
                for (index, item) in items.iter().enumerate() {
                    if let Some(field) = declarative_field_from_value(item, index) {
                        output.push(field);
                    }
                }
            } else {
                warnings.push(
                    "getSettingDefinitions() 使用了动态返回值，部分设置无法静态推断".to_string(),
                );
            }
        }
    }
    output
}

fn parse_declarative_array(
    array: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    warnings: &mut Vec<String>,
    output: &mut Vec<ExtractedField>,
) {
    let mut cursor = array.walk();
    for (index, child) in array.named_children(&mut cursor).enumerate() {
        if child.kind() == "object" {
            if let Some(field) = declarative_field_from_object(child, source, constants, index) {
                output.push(field);
            } else {
                warnings.push("存在无法静态解析的声明式设置项".to_string());
            }
        } else if let Some(value) = static_value(child, source, constants, 0) {
            if let Some(field) = declarative_field_from_value(&value, index) {
                output.push(field);
            }
        }
    }
}

fn declarative_field_from_object(
    object: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    index: usize,
) -> Option<ExtractedField> {
    let name = object_property_value(object, "name", source, constants).and_then(value_to_string);
    let description =
        object_property_value(object, "desc", source, constants).and_then(value_to_string);
    let page_path = object_property_value(object, "page", source, constants)
        .or_else(|| object_property_value(object, "pagePath", source, constants))
        .map(|value| value_to_string_vec(&value))
        .unwrap_or_default();
    let top_level_type =
        object_property_value(object, "type", source, constants).and_then(value_to_string);
    let control_node = object_property_node(object, "control", source);
    let has_render = object_property_node(object, "render", source).is_some();
    let has_action = object_property_node(object, "action", source).is_some();

    if top_level_type.as_deref() == Some("heading") {
        return Some(ExtractedField {
            field: new_field(
                format!("declarative-heading-{index}"),
                None,
                name.unwrap_or_else(|| "设置".to_string()),
                description,
                PluginSettingControl::Heading,
                PluginSettingSource::Declarative,
                PluginSettingConfidence::Exact,
            ),
            page_path,
        });
    }

    let mut field_warnings = Vec::new();
    let mut options = Vec::new();
    let mut placeholder = None;
    let mut min = None;
    let mut max = None;
    let mut step = None;
    let mut default_value = None;
    let (control, key) = if let Some(control_node) = control_node {
        let control_type = object_property_value(control_node, "type", source, constants)
            .and_then(value_to_string)
            .unwrap_or_else(|| "unsupported".to_string());
        let key =
            object_property_value(control_node, "key", source, constants).and_then(value_to_string);
        placeholder = object_property_value(control_node, "placeholder", source, constants)
            .and_then(value_to_string);
        default_value = object_property_value(control_node, "defaultValue", source, constants);
        min = object_property_number(control_node, "min", source, constants);
        max = object_property_number(control_node, "max", source, constants);
        step = object_property_number(control_node, "step", source, constants);
        if let Some(value) = object_property_value(control_node, "options", source, constants) {
            options = options_from_value(&value);
        }
        (control_from_declarative_type(&control_type), key)
    } else {
        if has_render || has_action {
            field_warnings.push("该设置依赖插件回调，只能在 Obsidian 中操作".to_string());
        }
        (PluginSettingControl::Unsupported, None)
    };

    let path = key.as_deref().map(pointer_for_key);
    let fallback_name = key
        .as_deref()
        .map(humanize_key)
        .unwrap_or_else(|| "自定义设置".to_string());
    let read_only = path.is_none() || matches!(control, PluginSettingControl::Unsupported);
    let mut field = new_field(
        format!(
            "declarative-{}",
            path.as_deref().unwrap_or(&index.to_string())
        ),
        path,
        name.unwrap_or(fallback_name),
        description,
        control,
        PluginSettingSource::Declarative,
        if read_only {
            PluginSettingConfidence::Inferred
        } else {
            PluginSettingConfidence::Exact
        },
    );
    field.options = options;
    field.placeholder = placeholder;
    field.min = min;
    field.max = max;
    field.step = step;
    field.default_value = default_value;
    field.read_only = read_only;
    field.warnings = field_warnings;
    field.support = classify_field_support(&field, has_action && control_node.is_none());
    Some(ExtractedField { field, page_path })
}

fn declarative_field_from_value(value: &Value, index: usize) -> Option<ExtractedField> {
    let object = value.as_object()?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let description = object
        .get("desc")
        .and_then(Value::as_str)
        .map(str::to_string);
    let page_path = object
        .get("page")
        .or_else(|| object.get("pagePath"))
        .map(value_to_string_vec)
        .unwrap_or_default();
    if object.get("type").and_then(Value::as_str) == Some("heading") {
        return Some(ExtractedField {
            field: new_field(
                format!("declarative-heading-{index}"),
                None,
                name.unwrap_or_else(|| "设置".to_string()),
                description,
                PluginSettingControl::Heading,
                PluginSettingSource::Declarative,
                PluginSettingConfidence::Exact,
            ),
            page_path,
        });
    }
    let control_object = object.get("control").and_then(Value::as_object);
    let control_type = control_object
        .and_then(|control| control.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("unsupported");
    let key = control_object
        .and_then(|control| control.get("key"))
        .and_then(Value::as_str);
    let path = key.map(pointer_for_key);
    let control = control_from_declarative_type(control_type);
    let read_only = path.is_none() || matches!(control, PluginSettingControl::Unsupported);
    let mut field = new_field(
        format!(
            "declarative-{}",
            path.as_deref().unwrap_or(&index.to_string())
        ),
        path,
        name.unwrap_or_else(|| {
            key.map(humanize_key)
                .unwrap_or_else(|| "自定义设置".to_string())
        }),
        description,
        control,
        PluginSettingSource::Declarative,
        if read_only {
            PluginSettingConfidence::Inferred
        } else {
            PluginSettingConfidence::Exact
        },
    );
    if let Some(control_object) = control_object {
        field.options = control_object
            .get("options")
            .map(options_from_value)
            .unwrap_or_default();
        field.placeholder = control_object
            .get("placeholder")
            .and_then(Value::as_str)
            .map(str::to_string);
        field.min = control_object.get("min").and_then(Value::as_f64);
        field.max = control_object.get("max").and_then(Value::as_f64);
        field.step = control_object.get("step").and_then(Value::as_f64);
        field.default_value = control_object.get("defaultValue").cloned();
    }
    field.read_only = read_only;
    field.support = classify_field_support(
        &field,
        object.get("action").is_some() && control_object.is_none(),
    );
    Some(ExtractedField { field, page_path })
}

fn extract_imperative_fields<'tree>(
    setting_tabs: &[RegisteredSettingTab<'tree>],
    source: &str,
    constants: &HashMap<String, Value>,
    configuration: Option<&Value>,
    program_index: &SettingsProgramIndex<'tree>,
    warnings: &mut Vec<String>,
) -> Vec<ExtractedField> {
    let mut statements = BTreeMap::<(usize, String), ReachableSetting<'tree>>::new();
    let mut found_display = false;
    let mut followed_dynamic_renderers = false;
    for tab in setting_tabs {
        let methods = class_methods(tab.class_node, source);
        let Some(display) = methods.get("display").copied() else {
            continue;
        };
        found_display = true;
        let mut initial_containers = BTreeMap::new();
        initial_containers.insert("containerEl".to_string(), Vec::new());
        let mut worklist = vec![ReachableScope {
            node: display,
            containers: initial_containers,
        }];
        let mut visited = BTreeSet::new();

        while let Some(reachable) = worklist.pop() {
            let visit_key = (
                reachable.node.start_byte(),
                reachable
                    .containers
                    .iter()
                    .map(|(name, path)| format!("{name}:{}", path.join("/")))
                    .collect::<Vec<_>>()
                    .join("|"),
            );
            if !visited.insert(visit_key) {
                continue;
            }

            let environment = discover_settings_containers(
                reachable.node,
                source,
                constants,
                reachable.containers,
            );
            collect_reachable_setting_statements(
                reachable.node,
                source,
                constants,
                configuration,
                &program_index.aliases,
                &environment.containers,
                &environment.container_maps,
                &mut statements,
            );
            enqueue_reachable_methods(
                reachable.node,
                source,
                constants,
                &methods,
                program_index,
                &environment.containers,
                &environment.container_maps,
                &mut worklist,
                &mut followed_dynamic_renderers,
            );
        }
    }

    if !found_display {
        return Vec::new();
    }

    let mut output = Vec::new();
    for (index, (_, reachable)) in statements.into_iter().enumerate() {
        if let Some(mut field) = imperative_field_from_statement(
            reachable.statement,
            source,
            constants,
            configuration,
            program_index,
            index,
        ) {
            field.page_path = reachable.page_path;
            if reachable.condition_unknown {
                field.field.read_only = true;
                field
                    .field
                    .warnings
                    .push("显示条件无法静态求值，已设为只读".to_string());
            }
            output.push(field);
        }
    }
    if followed_dynamic_renderers {
        warnings.push(
            "已按 settingsTabRenderer 注册关系分析提供商页；运行时生成的控件可能未完整显示"
                .to_string(),
        );
    }
    if output.is_empty() && source.contains("PluginSettingTab") && source.contains("new Setting") {
        warnings.push("检测到插件设置页，但没有找到可安全映射的标准设置行".to_string());
    }
    output
}

#[derive(Default)]
struct ContainerEnvironment {
    containers: BTreeMap<String, Vec<String>>,
    container_maps: BTreeSet<String>,
}

fn discover_settings_containers(
    scope: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    initial: BTreeMap<String, Vec<String>>,
) -> ContainerEnvironment {
    let mut environment = ContainerEnvironment {
        containers: initial,
        container_maps: BTreeSet::new(),
    };
    let mut declarators = Vec::new();
    let mut calls = Vec::new();
    collect_scope_nodes(scope, &mut |node| match node.kind() {
        "variable_declarator" => declarators.push(node),
        "call_expression" => calls.push(node),
        _ => {}
    });

    for _ in 0..8 {
        let mut changed = false;
        for declarator in &declarators {
            let Some(name) = declarator.child_by_field_name("name") else {
                continue;
            };
            let Some(value) = declarator.child_by_field_name("value") else {
                continue;
            };
            if name.kind() == "object_pattern" && node_text(value, source) == "this" {
                for alias in container_el_pattern_aliases(node_text(name, source)) {
                    if environment.containers.insert(alias, Vec::new()).is_none() {
                        changed = true;
                    }
                }
                continue;
            }
            if name.kind() != "identifier" {
                continue;
            }
            let name = node_text(name, source).to_string();
            if is_map_constructor(value, source) {
                continue;
            }
            if let Some(page_path) = settings_container_page(
                value,
                source,
                constants,
                &environment.containers,
                &environment.container_maps,
            ) {
                if environment.containers.insert(name, page_path).is_none() {
                    changed = true;
                }
            }
        }
        for call in &calls {
            if call_method_name(*call, source).as_deref() != Some("set") {
                continue;
            }
            let Some(receiver) = call_receiver_identifier(*call, source) else {
                continue;
            };
            let arguments = call_arguments(*call);
            let Some(value) = arguments.get(1).copied() else {
                continue;
            };
            if settings_container_page(
                value,
                source,
                constants,
                &environment.containers,
                &environment.container_maps,
            )
            .is_some()
                && environment.container_maps.insert(receiver)
            {
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    environment
}

fn container_el_pattern_aliases(pattern: &str) -> Vec<String> {
    pattern
        .trim_matches(|character| matches!(character, '{' | '}'))
        .split(',')
        .filter_map(|entry| {
            let mut parts = entry.split(':').map(str::trim);
            let key = parts.next()?;
            if key != "containerEl" {
                return None;
            }
            Some(parts.next().unwrap_or(key).to_string())
        })
        .collect()
}

fn is_map_constructor(node: Node<'_>, source: &str) -> bool {
    node.kind() == "new_expression"
        && node
            .child_by_field_name("constructor")
            .map(|constructor| node_text(constructor, source).ends_with("Map"))
            .unwrap_or(false)
}

fn settings_container_page(
    expression: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    containers: &BTreeMap<String, Vec<String>>,
    container_maps: &BTreeSet<String>,
) -> Option<Vec<String>> {
    match expression.kind() {
        "identifier" => containers.get(node_text(expression, source)).cloned(),
        "member_expression" => {
            let text = node_text(expression, source);
            (text == "this.containerEl").then(Vec::new)
        }
        "parenthesized_expression" => expression.named_child(0).and_then(|child| {
            settings_container_page(child, source, constants, containers, container_maps)
        }),
        "call_expression" => {
            let method = call_method_name(expression, source)?;
            if matches!(method.as_str(), "createDiv" | "createEl" | "createSpan") {
                let receiver = call_receiver(expression)?;
                return settings_container_page(
                    receiver,
                    source,
                    constants,
                    containers,
                    container_maps,
                );
            }
            if method == "get" {
                let receiver = call_receiver_identifier(expression, source)?;
                if !container_maps.contains(&receiver) {
                    return None;
                }
                let page = call_arguments(expression)
                    .first()
                    .and_then(|argument| static_value(*argument, source, constants, 0))
                    .and_then(value_to_string);
                return Some(page.into_iter().collect());
            }
            None
        }
        _ => None,
    }
}

fn call_receiver(call: Node<'_>) -> Option<Node<'_>> {
    let function = call.child_by_field_name("function")?;
    if function.kind() != "member_expression" {
        return None;
    }
    function.child_by_field_name("object")
}

fn call_receiver_identifier(call: Node<'_>, source: &str) -> Option<String> {
    let receiver = call_receiver(call)?;
    (receiver.kind() == "identifier").then(|| node_text(receiver, source).to_string())
}

#[allow(clippy::too_many_arguments)]
fn collect_reachable_setting_statements<'tree>(
    scope: Node<'tree>,
    source: &str,
    constants: &HashMap<String, Value>,
    configuration: Option<&Value>,
    aliases: &ObsidianAliases,
    containers: &BTreeMap<String, Vec<String>>,
    container_maps: &BTreeSet<String>,
    output: &mut BTreeMap<(usize, String), ReachableSetting<'tree>>,
) {
    collect_scope_nodes(scope, &mut |node| {
        if node.kind() != "new_expression" || !is_setting_constructor(node, source, aliases) {
            return;
        }
        let Some(arguments) = node.child_by_field_name("arguments") else {
            return;
        };
        let Some(container) = arguments.named_child(0) else {
            return;
        };
        let Some(page_path) =
            settings_container_page(container, source, constants, containers, container_maps)
        else {
            return;
        };
        let visibility = conditional_visibility(node, scope, source, constants, configuration);
        if visibility == ConditionalVisibility::Hidden {
            return;
        }
        let statement = expression_scope(node);
        let key = (statement.start_byte(), page_path.join("/"));
        output.insert(
            key,
            ReachableSetting {
                statement,
                page_path,
                condition_unknown: visibility == ConditionalVisibility::Unknown,
            },
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn enqueue_reachable_methods<'tree>(
    scope: Node<'tree>,
    source: &str,
    constants: &HashMap<String, Value>,
    methods: &BTreeMap<String, Node<'tree>>,
    program_index: &SettingsProgramIndex<'tree>,
    containers: &BTreeMap<String, Vec<String>>,
    container_maps: &BTreeSet<String>,
    worklist: &mut Vec<ReachableScope<'tree>>,
    followed_dynamic_renderers: &mut bool,
) {
    collect_scope_nodes(scope, &mut |call| {
        if call.kind() != "call_expression" {
            return;
        }
        let function = call.child_by_field_name("function");
        let callee = match function.map(|node| node.kind()) {
            Some("member_expression")
                if call_receiver(call)
                    .map(|receiver| node_text(receiver, source) == "this")
                    .unwrap_or(false) =>
            {
                call_method_name(call, source).and_then(|name| methods.get(&name).copied())
            }
            Some("identifier") => function.and_then(|function| {
                program_index
                    .functions
                    .get(node_text(function, source))
                    .copied()
            }),
            _ => None,
        };
        if let Some(callee) = callee {
            if callee == scope {
                return;
            }
            let next_containers = callable_container_bindings(
                callee,
                &call_arguments(call),
                source,
                constants,
                containers,
                container_maps,
            );
            let same_class_method = methods.values().any(|method| *method == callee);
            if same_class_method || !next_containers.is_empty() {
                worklist.push(ReachableScope {
                    node: callee,
                    containers: next_containers,
                });
            }
        }
    });

    if !node_text(scope, source).contains("getSettingsTabRenderer") {
        return;
    }
    let mut renderer_containers = Vec::new();
    collect_scope_nodes(scope, &mut |call| {
        if call.kind() != "call_expression"
            || call_method_name(call, source).as_deref() != Some("render")
        {
            return;
        }
        let Some(argument) = call_arguments(call).first().copied() else {
            return;
        };
        if settings_container_page(argument, source, constants, containers, container_maps)
            .is_some()
        {
            renderer_containers.push(argument);
        }
    });
    if renderer_containers.is_empty() {
        return;
    }
    for renderer in &program_index.renderers {
        let Some(first_parameter) = callable_parameters(renderer.render_node).first().copied()
        else {
            continue;
        };
        if first_parameter.kind() != "identifier" {
            continue;
        }
        let mut next_containers = BTreeMap::new();
        next_containers.insert(
            node_text(first_parameter, source).to_string(),
            vec![renderer.page_name.clone()],
        );
        worklist.push(ReachableScope {
            node: renderer.render_node,
            containers: next_containers,
        });
        *followed_dynamic_renderers = true;
    }
}

fn callable_parameters(callable: Node<'_>) -> Vec<Node<'_>> {
    if let Some(parameter) = callable.child_by_field_name("parameter") {
        return vec![parameter];
    }
    let Some(parameters) = callable.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = parameters.walk();
    parameters.named_children(&mut cursor).collect()
}

fn callable_container_bindings(
    callable: Node<'_>,
    arguments: &[Node<'_>],
    source: &str,
    constants: &HashMap<String, Value>,
    containers: &BTreeMap<String, Vec<String>>,
    container_maps: &BTreeSet<String>,
) -> BTreeMap<String, Vec<String>> {
    let mut bindings = BTreeMap::new();
    for (parameter, argument) in callable_parameters(callable).into_iter().zip(arguments) {
        if parameter.kind() == "identifier" {
            let parameter_name = node_text(parameter, source);
            if let Some(page_path) =
                settings_container_page(*argument, source, constants, containers, container_maps)
            {
                bindings.insert(parameter_name.to_string(), page_path);
            }
            if argument.kind() == "object" {
                collect_destructured_container_bindings(
                    callable,
                    parameter_name,
                    *argument,
                    source,
                    constants,
                    containers,
                    container_maps,
                    &mut bindings,
                );
            }
        } else if parameter.kind() == "object_pattern" && argument.kind() == "object" {
            bind_object_pattern_containers(
                parameter,
                *argument,
                source,
                constants,
                containers,
                container_maps,
                &mut bindings,
            );
        }
    }
    bindings
}

#[allow(clippy::too_many_arguments)]
fn collect_destructured_container_bindings(
    callable: Node<'_>,
    parameter_name: &str,
    argument: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    containers: &BTreeMap<String, Vec<String>>,
    container_maps: &BTreeSet<String>,
    bindings: &mut BTreeMap<String, Vec<String>>,
) {
    collect_scope_nodes(callable, &mut |node| {
        if node.kind() != "variable_declarator" {
            return;
        }
        let Some(pattern) = node.child_by_field_name("name") else {
            return;
        };
        let Some(value) = node.child_by_field_name("value") else {
            return;
        };
        if pattern.kind() != "object_pattern" || node_text(value, source) != parameter_name {
            return;
        }
        bind_object_pattern_containers(
            pattern,
            argument,
            source,
            constants,
            containers,
            container_maps,
            bindings,
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn bind_object_pattern_containers(
    pattern: Node<'_>,
    argument: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    containers: &BTreeMap<String, Vec<String>>,
    container_maps: &BTreeSet<String>,
    bindings: &mut BTreeMap<String, Vec<String>>,
) {
    let mut cursor = pattern.walk();
    for child in pattern.named_children(&mut cursor) {
        let Some((property, binding)) = object_pattern_entry(child, source) else {
            continue;
        };
        let Some(value) = object_property_expression(argument, &property, source) else {
            continue;
        };
        if let Some(page_path) =
            settings_container_page(value, source, constants, containers, container_maps)
        {
            bindings.insert(binding, page_path);
        }
    }
}

fn object_pattern_entry(node: Node<'_>, source: &str) -> Option<(String, String)> {
    match node.kind() {
        "shorthand_property_identifier_pattern" | "identifier" => {
            let name = node_text(node, source).to_string();
            Some((name.clone(), name))
        }
        "pair_pattern" | "pair" => {
            let key = property_key(node.child_by_field_name("key")?, source)?;
            let mut value = node.child_by_field_name("value")?;
            if value.kind() == "assignment_pattern" {
                value = value.child_by_field_name("left")?;
            }
            (value.kind() == "identifier").then(|| (key, node_text(value, source).to_string()))
        }
        _ => None,
    }
}

fn object_property_expression<'tree>(
    object: Node<'tree>,
    expected: &str,
    source: &str,
) -> Option<Node<'tree>> {
    let mut cursor = object.walk();
    let expression = object.named_children(&mut cursor).find_map(|child| {
        if matches!(
            child.kind(),
            "shorthand_property_identifier" | "shorthand_property_identifier_pattern"
        ) && node_text(child, source) == expected
        {
            return Some(child);
        }
        if child.kind() != "pair" {
            return None;
        }
        let key = property_key(child.child_by_field_name("key")?, source)?;
        (key == expected)
            .then(|| child.child_by_field_name("value"))
            .flatten()
    });
    expression
}

fn conditional_visibility(
    node: Node<'_>,
    scope: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    configuration: Option<&Value>,
) -> ConditionalVisibility {
    let mut visibility = ConditionalVisibility::Visible;
    let mut child = node;
    while child != scope {
        let Some(parent) = child.parent() else {
            break;
        };
        if parent.kind() == "if_statement" {
            let Some(condition) = parent.child_by_field_name("condition") else {
                visibility = ConditionalVisibility::Unknown;
                child = parent;
                continue;
            };
            let Some(result) = evaluate_condition(condition, source, constants, configuration)
            else {
                visibility = ConditionalVisibility::Unknown;
                child = parent;
                continue;
            };
            let in_consequence = parent.child_by_field_name("consequence") == Some(child);
            let in_alternative = parent.child_by_field_name("alternative") == Some(child);
            if (in_consequence && !result) || (in_alternative && result) {
                return ConditionalVisibility::Hidden;
            }
        }
        child = parent;
    }
    visibility
}

fn evaluate_condition(
    node: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    configuration: Option<&Value>,
) -> Option<bool> {
    condition_value(node, source, constants, configuration).map(|value| value_is_truthy(&value))
}

fn condition_value(
    node: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    configuration: Option<&Value>,
) -> Option<Value> {
    if let Some(pointer) = pointer_from_expression(node, source, constants) {
        return configuration
            .and_then(|value| value.pointer(&pointer))
            .cloned();
    }
    if let Some(value) = static_value(node, source, constants, 0) {
        return Some(value);
    }
    match node.kind() {
        "parenthesized_expression" => {
            condition_value(node.named_child(0)?, source, constants, configuration)
        }
        "unary_expression" => {
            let argument = node.child_by_field_name("argument")?;
            let value = condition_value(argument, source, constants, configuration)?;
            node_text(node, source)
                .trim_start()
                .starts_with('!')
                .then(|| Value::Bool(!value_is_truthy(&value)))
        }
        "binary_expression" => {
            let left = node.child_by_field_name("left")?;
            let right = node.child_by_field_name("right")?;
            let operator = source.get(left.end_byte()..right.start_byte())?.trim();
            let left_value = condition_value(left, source, constants, configuration);
            match operator {
                "&&" => {
                    let left_value = left_value?;
                    if !value_is_truthy(&left_value) {
                        Some(left_value)
                    } else {
                        condition_value(right, source, constants, configuration)
                    }
                }
                "||" => {
                    let left_value = left_value?;
                    if value_is_truthy(&left_value) {
                        Some(left_value)
                    } else {
                        condition_value(right, source, constants, configuration)
                    }
                }
                "??" => {
                    let left_value = left_value?;
                    if left_value.is_null() {
                        condition_value(right, source, constants, configuration)
                    } else {
                        Some(left_value)
                    }
                }
                "==" | "===" | "!=" | "!==" => {
                    let right_value = condition_value(right, source, constants, configuration)?;
                    let equal = left_value? == right_value;
                    Some(Value::Bool(if matches!(operator, "!=" | "!==") {
                        !equal
                    } else {
                        equal
                    }))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn value_is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn collect_scope_nodes<'tree>(root: Node<'tree>, visit: &mut impl FnMut(Node<'tree>)) {
    fn walk<'tree>(node: Node<'tree>, root: Node<'tree>, visit: &mut impl FnMut(Node<'tree>)) {
        if node != root
            && matches!(
                node.kind(),
                "method_definition"
                    | "function_declaration"
                    | "function_expression"
                    | "arrow_function"
                    | "generator_function_declaration"
                    | "generator_function"
            )
        {
            return;
        }
        visit(node);
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(child, root, visit);
        }
    }
    walk(root, root, visit);
}

#[derive(Default)]
struct IndirectPathBinding {
    path: Option<String>,
    read_paths: Vec<String>,
    path_options: Vec<PluginSettingPathOption>,
    dynamic_path: bool,
    complex_write: bool,
}

fn infer_indirect_path_binding(
    statement: Node<'_>,
    control_calls: &[Node<'_>],
    source: &str,
    constants: &HashMap<String, Value>,
    configuration: Option<&Value>,
    program_index: &SettingsProgramIndex<'_>,
) -> IndirectPathBinding {
    let Some(scope) = enclosing_callable(statement) else {
        return IndirectPathBinding::default();
    };
    let Some((getter_name, property)) =
        setting_getter_and_property(control_calls, scope, statement.start_byte(), source)
    else {
        return infer_local_dynamic_map_binding(
            statement,
            control_calls,
            scope,
            source,
            constants,
            configuration,
            program_index,
        );
    };
    let Some(callback) = control_calls.iter().find_map(|call| {
        (call_method_name(*call, source).as_deref() == Some("onChange"))
            .then(|| call_arguments(*call).first().copied())
            .flatten()
    }) else {
        return IndirectPathBinding::default();
    };
    let Some(update) = callback_update_binding(callback, source, program_index)
        .or_else(|| deferred_control_update_binding(statement, &property, source, program_index))
    else {
        return IndirectPathBinding::default();
    };
    if update.property != property {
        return IndirectPathBinding::default();
    }
    let Some(write_target) = provider_write_target(&update, source, constants, program_index)
    else {
        return IndirectPathBinding::default();
    };

    let mut binding = IndirectPathBinding {
        complex_write: update.complex_write || write_target.complex_write,
        ..IndirectPathBinding::default()
    };
    if let Some(path) = write_target.path {
        binding.read_paths = getter_read_paths(
            &getter_name,
            &property,
            &path,
            source,
            constants,
            program_index,
        );
        binding.path = Some(path);
    } else if let Some(prefix) = write_target.dynamic_prefix {
        binding.dynamic_path = true;
        binding.path_options = enumerate_path_options(configuration, &prefix);
    }
    binding
}

fn infer_local_dynamic_map_binding(
    statement: Node<'_>,
    control_calls: &[Node<'_>],
    scope: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    configuration: Option<&Value>,
    program_index: &SettingsProgramIndex<'_>,
) -> IndirectPathBinding {
    let Some((_, map_property)) =
        dynamic_getter_map_property(control_calls, scope, statement.start_byte(), source)
    else {
        return IndirectPathBinding::default();
    };
    let Some(callback) = control_calls.iter().find_map(|call| {
        (call_method_name(*call, source).as_deref() == Some("onChange"))
            .then(|| call_arguments(*call).first().copied())
            .flatten()
    }) else {
        return IndirectPathBinding::default();
    };
    let parameters = callback_parameter_names(callback, source);
    let Some(local_helper_name) = collect_called_function_names(callback, source)
        .into_iter()
        .find(|name| {
            local_initializer(scope, name, statement.start_byte(), source)
                .is_some_and(|node| matches!(node.kind(), "arrow_function" | "function_expression"))
        })
    else {
        return IndirectPathBinding::default();
    };
    let Some(local_helper) =
        local_initializer(scope, &local_helper_name, statement.start_byte(), source)
    else {
        return IndirectPathBinding::default();
    };
    if !matches!(
        local_helper.kind(),
        "arrow_function" | "function_expression"
    ) {
        return IndirectPathBinding::default();
    }
    let helper_parameters = callable_parameter_names(local_helper, source);
    let Some(helper_value_parameter) = helper_parameters.first() else {
        return IndirectPathBinding::default();
    };
    if parameters.is_empty()
        || !callback_calls_helper_with_parameter(callback, &local_helper_name, &parameters, source)
    {
        return IndirectPathBinding::default();
    }
    let Some(map_variable) =
        local_helper_dynamic_map_variable(local_helper, helper_value_parameter, source)
    else {
        return IndirectPathBinding::default();
    };
    let Some(update) = local_helper_provider_update(
        local_helper,
        &map_property,
        &map_variable,
        source,
        program_index,
    ) else {
        return IndirectPathBinding::default();
    };
    let Some(target) = provider_write_target(&update, source, constants, program_index) else {
        return IndirectPathBinding::default();
    };
    let Some(prefix) = target.path else {
        return IndirectPathBinding::default();
    };
    IndirectPathBinding {
        path_options: enumerate_path_options(configuration, &prefix),
        dynamic_path: true,
        complex_write: true,
        ..IndirectPathBinding::default()
    }
}

fn dynamic_getter_map_property(
    control_calls: &[Node<'_>],
    scope: Node<'_>,
    before_byte: usize,
    source: &str,
) -> Option<(String, String)> {
    let expression = control_calls.iter().find_map(|call| {
        (call_method_name(*call, source).as_deref() == Some("setValue"))
            .then(|| call_arguments(*call).first().copied())
            .flatten()
    })?;
    let expression = if expression.kind() == "identifier" {
        local_initializer(scope, node_text(expression, source), before_byte, source)?
    } else {
        expression
    };
    let mut result = None;
    collect_nodes(expression, &mut |subscript| {
        if result.is_some() || subscript.kind() != "subscript_expression" {
            return;
        }
        let Some(object) = subscript.child_by_field_name("object") else {
            return;
        };
        if object.kind() != "member_expression" {
            return;
        }
        let Some(settings_object) = object.child_by_field_name("object") else {
            return;
        };
        let Some(property) = object.child_by_field_name("property") else {
            return;
        };
        if settings_object.kind() != "identifier" {
            return;
        }
        let Some(initializer) = local_initializer(
            scope,
            node_text(settings_object, source),
            before_byte,
            source,
        ) else {
            return;
        };
        if initializer.kind() != "call_expression" {
            return;
        }
        let Some(getter_name) = call_function_identifier(initializer, source) else {
            return;
        };
        result = Some((getter_name, node_text(property, source).to_string()));
    });
    result
}

fn collect_called_function_names(node: Node<'_>, source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    collect_nodes(node, &mut |call| {
        if call.kind() == "call_expression" {
            if let Some(name) = call_function_identifier(call, source) {
                names.insert(name);
            }
        }
    });
    names
}

fn callback_calls_helper_with_parameter(
    callback: Node<'_>,
    helper_name: &str,
    parameters: &BTreeSet<String>,
    source: &str,
) -> bool {
    let mut found = false;
    collect_nodes(callback, &mut |call| {
        if found
            || call.kind() != "call_expression"
            || call_function_identifier(call, source).as_deref() != Some(helper_name)
        {
            return;
        }
        found = call_arguments(call).iter().any(|argument| {
            argument.kind() == "identifier" && parameters.contains(node_text(*argument, source))
        });
    });
    found
}

fn local_helper_dynamic_map_variable(
    helper: Node<'_>,
    value_parameter: &str,
    source: &str,
) -> Option<String> {
    let mut tainted = BTreeSet::from([value_parameter.to_string()]);
    for _ in 0..4 {
        let mut changed = false;
        collect_scope_nodes(helper, &mut |node| {
            let (name, value) = match node.kind() {
                "variable_declarator" => (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("value"),
                ),
                "assignment_expression" => (
                    node.child_by_field_name("left"),
                    node.child_by_field_name("right"),
                ),
                _ => return,
            };
            let (Some(name), Some(value)) = (name, value) else {
                return;
            };
            if name.kind() == "identifier"
                && expression_references_names(value, source, &tainted)
                && tainted.insert(node_text(name, source).to_string())
            {
                changed = true;
            }
        });
        if !changed {
            break;
        }
    }
    let mut result = None;
    collect_scope_nodes(helper, &mut |assignment| {
        if result.is_some() || assignment.kind() != "assignment_expression" {
            return;
        }
        let Some(left) = assignment.child_by_field_name("left") else {
            return;
        };
        let Some(right) = assignment.child_by_field_name("right") else {
            return;
        };
        if left.kind() != "subscript_expression"
            || !expression_references_names(right, source, &tainted)
        {
            return;
        }
        let Some(object) = left.child_by_field_name("object") else {
            return;
        };
        if object.kind() == "identifier" {
            result = Some(node_text(object, source).to_string());
        }
    });
    result
}

fn local_helper_provider_update(
    helper: Node<'_>,
    map_property: &str,
    map_variable: &str,
    source: &str,
    program_index: &SettingsProgramIndex<'_>,
) -> Option<CallbackUpdateBinding> {
    let names = BTreeSet::from([map_variable.to_string()]);
    let mut result = None;
    collect_scope_nodes(helper, &mut |call| {
        if result.is_some() || call.kind() != "call_expression" {
            return;
        }
        let Some(helper_name) = call_function_identifier(call, source) else {
            return;
        };
        if !program_index.functions.contains_key(&helper_name) {
            return;
        }
        for argument in call_arguments(call) {
            if argument.kind() != "object" {
                continue;
            }
            let Some(value) = object_property_expression(argument, map_property, source) else {
                continue;
            };
            if expression_references_names(value, source, &names) {
                result = Some(CallbackUpdateBinding {
                    helper_name: helper_name.clone(),
                    property: map_property.to_string(),
                    complex_write: true,
                });
                return;
            }
        }
    });
    result
}

fn enclosing_callable(mut node: Node<'_>) -> Option<Node<'_>> {
    while let Some(parent) = node.parent() {
        if matches!(
            parent.kind(),
            "method_definition" | "function_declaration" | "function_expression" | "arrow_function"
        ) {
            return Some(parent);
        }
        node = parent;
    }
    None
}

fn setting_getter_and_property(
    control_calls: &[Node<'_>],
    scope: Node<'_>,
    before_byte: usize,
    source: &str,
) -> Option<(String, String)> {
    let expression = control_calls.iter().find_map(|call| {
        (call_method_name(*call, source).as_deref() == Some("setValue"))
            .then(|| call_arguments(*call).first().copied())
            .flatten()
    })?;
    getter_call_and_property(expression, scope, before_byte, source, 0)
}

fn getter_call_and_property(
    expression: Node<'_>,
    scope: Node<'_>,
    before_byte: usize,
    source: &str,
    depth: usize,
) -> Option<(String, String)> {
    if depth > 8 {
        return None;
    }
    match expression.kind() {
        "identifier" => {
            let initializer =
                local_initializer(scope, node_text(expression, source), before_byte, source)?;
            getter_call_and_property(
                initializer,
                scope,
                initializer.start_byte(),
                source,
                depth + 1,
            )
        }
        "member_expression" => {
            let object = expression.child_by_field_name("object")?;
            let property = expression.child_by_field_name("property")?;
            let property = node_text(property, source).to_string();
            if object.kind() != "identifier" {
                return None;
            }
            let initializer =
                local_initializer(scope, node_text(object, source), before_byte, source)?;
            if initializer.kind() == "call_expression" {
                return call_function_identifier(initializer, source).map(|name| (name, property));
            }
            getter_call_and_property(
                initializer,
                scope,
                initializer.start_byte(),
                source,
                depth + 1,
            )
        }
        "parenthesized_expression" => getter_call_and_property(
            expression.named_child(0)?,
            scope,
            before_byte,
            source,
            depth + 1,
        ),
        _ => None,
    }
}

fn local_initializer<'tree>(
    scope: Node<'tree>,
    name: &str,
    before_byte: usize,
    source: &str,
) -> Option<Node<'tree>> {
    let mut found = None;
    collect_scope_nodes(scope, &mut |node| {
        if node.kind() != "variable_declarator" || node.start_byte() >= before_byte {
            return;
        }
        let Some(binding) = node.child_by_field_name("name") else {
            return;
        };
        if binding.kind() != "identifier" || node_text(binding, source) != name {
            return;
        }
        if let Some(value) = node.child_by_field_name("value") {
            found = Some(value);
        }
    });
    found
}

fn call_function_identifier(call: Node<'_>, source: &str) -> Option<String> {
    let function = call.child_by_field_name("function")?;
    (function.kind() == "identifier").then(|| node_text(function, source).to_string())
}

#[derive(Default)]
struct CallbackUpdateBinding {
    helper_name: String,
    property: String,
    complex_write: bool,
}

fn callback_update_binding(
    callback: Node<'_>,
    source: &str,
    program_index: &SettingsProgramIndex<'_>,
) -> Option<CallbackUpdateBinding> {
    let parameters = callback_parameter_names(callback, source);
    let mut tainted = parameters.clone();
    for _ in 0..4 {
        let mut changed = false;
        collect_scope_nodes(callback, &mut |node| {
            let (name, value) = match node.kind() {
                "variable_declarator" => (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("value"),
                ),
                "assignment_expression" => (
                    node.child_by_field_name("left"),
                    node.child_by_field_name("right"),
                ),
                _ => return,
            };
            let (Some(name), Some(value)) = (name, value) else {
                return;
            };
            if name.kind() == "identifier"
                && expression_references_names(value, source, &tainted)
                && tainted.insert(node_text(name, source).to_string())
            {
                changed = true;
            }
        });
        if !changed {
            break;
        }
    }

    let mut result = None;
    collect_scope_nodes(callback, &mut |call| {
        if result.is_some() || call.kind() != "call_expression" {
            return;
        }
        let Some(helper_name) = call_function_identifier(call, source) else {
            return;
        };
        if !program_index.functions.contains_key(&helper_name) {
            return;
        }
        for argument in call_arguments(call) {
            if argument.kind() != "object" {
                continue;
            }
            let mut cursor = argument.walk();
            for child in argument.named_children(&mut cursor) {
                let (property, value, shorthand) = if child.kind() == "pair" {
                    let Some(key) = child
                        .child_by_field_name("key")
                        .and_then(|key| property_key(key, source))
                    else {
                        continue;
                    };
                    let Some(value) = child.child_by_field_name("value") else {
                        continue;
                    };
                    (key, value, false)
                } else if matches!(
                    child.kind(),
                    "shorthand_property_identifier" | "shorthand_property_identifier_pattern"
                ) {
                    (node_text(child, source).to_string(), child, true)
                } else {
                    continue;
                };
                if !(expression_references_names(value, source, &tainted)
                    || shorthand && tainted.contains(&property))
                {
                    continue;
                }
                let direct = if shorthand {
                    parameters.contains(&property)
                } else {
                    value.kind() == "identifier" && parameters.contains(node_text(value, source))
                };
                result = Some(CallbackUpdateBinding {
                    helper_name: helper_name.clone(),
                    property,
                    complex_write: !direct,
                });
                return;
            }
        }
    });
    result
}

fn deferred_control_update_binding(
    statement: Node<'_>,
    expected_property: &str,
    source: &str,
    program_index: &SettingsProgramIndex<'_>,
) -> Option<CallbackUpdateBinding> {
    let mut pending_names = BTreeSet::new();
    collect_nodes(statement, &mut |call| {
        if call.kind() != "call_expression"
            || call_method_name(call, source).as_deref() != Some("onChange")
        {
            return;
        }
        let Some(callback) = call_arguments(call).first().copied() else {
            return;
        };
        let parameters = callback_parameter_names(callback, source);
        collect_nodes(callback, &mut |node| {
            let (name, value) = match node.kind() {
                "variable_declarator" => (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("value"),
                ),
                "assignment_expression" => (
                    node.child_by_field_name("left"),
                    node.child_by_field_name("right"),
                ),
                _ => return,
            };
            let (Some(name), Some(value)) = (name, value) else {
                return;
            };
            if name.kind() == "identifier"
                && expression_references_names(value, source, &parameters)
            {
                pending_names.insert(node_text(name, source).to_string());
            }
        });
    });
    if pending_names.is_empty() {
        return None;
    }

    let mut result = None;
    collect_nodes(statement, &mut |call| {
        if result.is_some() || call.kind() != "call_expression" {
            return;
        }
        let Some(helper_name) = call_function_identifier(call, source) else {
            return;
        };
        if !program_index.functions.contains_key(&helper_name) {
            return;
        }
        for argument in call_arguments(call) {
            if argument.kind() != "object" {
                continue;
            }
            let mut cursor = argument.walk();
            for child in argument.named_children(&mut cursor) {
                let (property, value, shorthand) = if child.kind() == "pair" {
                    let Some(key) = child
                        .child_by_field_name("key")
                        .and_then(|key| property_key(key, source))
                    else {
                        continue;
                    };
                    let Some(value) = child.child_by_field_name("value") else {
                        continue;
                    };
                    (key, value, false)
                } else if matches!(
                    child.kind(),
                    "shorthand_property_identifier" | "shorthand_property_identifier_pattern"
                ) {
                    (node_text(child, source).to_string(), child, true)
                } else {
                    continue;
                };
                if property != expected_property {
                    continue;
                }
                let uses_pending = if shorthand {
                    pending_names.contains(&property)
                } else {
                    expression_references_names(value, source, &pending_names)
                };
                if uses_pending {
                    result = Some(CallbackUpdateBinding {
                        helper_name: helper_name.clone(),
                        property,
                        complex_write: true,
                    });
                    return;
                }
            }
        }
    });
    result
}

fn expression_references_names(
    expression: Node<'_>,
    source: &str,
    names: &BTreeSet<String>,
) -> bool {
    let mut found = false;
    collect_nodes(expression, &mut |node| {
        if !found && node.kind() == "identifier" && names.contains(node_text(node, source)) {
            found = true;
        }
    });
    found
}

#[derive(Default)]
struct ProviderWriteTarget {
    path: Option<String>,
    dynamic_prefix: Option<String>,
    complex_write: bool,
}

fn provider_write_target(
    update: &CallbackUpdateBinding,
    source: &str,
    constants: &HashMap<String, Value>,
    program_index: &SettingsProgramIndex<'_>,
) -> Option<ProviderWriteTarget> {
    let helper = *program_index.functions.get(&update.helper_name)?;
    let parameters = callable_parameter_names(helper, source);
    let updates_parameter = parameters.get(1)?;
    let mut result = None;
    collect_scope_nodes(helper, &mut |call| {
        if result.is_some() || call.kind() != "call_expression" {
            return;
        }
        let Some(setter_name) = call_function_identifier(call, source) else {
            return;
        };
        let Some(setter) = program_index.functions.get(&setter_name).copied() else {
            return;
        };
        if !is_provider_config_setter(setter, source) {
            return;
        }
        let arguments = call_arguments(call);
        let Some(provider_id) = arguments
            .get(1)
            .and_then(|argument| static_value(*argument, source, constants, 0))
            .and_then(value_to_string)
        else {
            return;
        };
        let Some(config_argument) = arguments.get(2).copied() else {
            return;
        };
        let config_object =
            resolve_object_expression(config_argument, helper, call.start_byte(), source)
                .unwrap_or(config_argument);
        let update_flow = object_update_flow(
            config_object,
            helper,
            updates_parameter,
            &update.property,
            source,
            0,
        );
        if update_flow != UpdateFlow::None {
            result = Some(ProviderWriteTarget {
                path: Some(pointer_from_segments(&[
                    "providerConfigs".to_string(),
                    provider_id,
                    update.property.clone(),
                ])),
                complex_write: update_flow == UpdateFlow::Complex,
                ..ProviderWriteTarget::default()
            });
            return;
        }
        let Some(dynamic_output) = dynamic_output_property(
            helper,
            updates_parameter,
            &update.property,
            config_object,
            source,
        ) else {
            return;
        };
        result = Some(ProviderWriteTarget {
            dynamic_prefix: Some(pointer_from_segments(&[
                "providerConfigs".to_string(),
                provider_id,
                dynamic_output,
            ])),
            complex_write: true,
            ..ProviderWriteTarget::default()
        });
    });
    result
}

fn resolve_object_expression<'tree>(
    expression: Node<'tree>,
    scope: Node<'tree>,
    before_byte: usize,
    source: &str,
) -> Option<Node<'tree>> {
    match expression.kind() {
        "object" => Some(expression),
        "identifier" => {
            let initializer =
                local_initializer(scope, node_text(expression, source), before_byte, source)?;
            (initializer.kind() == "object").then_some(initializer)
        }
        "parenthesized_expression" => {
            resolve_object_expression(expression.named_child(0)?, scope, before_byte, source)
        }
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UpdateFlow {
    None,
    Direct,
    Complex,
}

fn object_update_flow(
    object: Node<'_>,
    helper: Node<'_>,
    updates_parameter: &str,
    property: &str,
    source: &str,
    depth: usize,
) -> UpdateFlow {
    if object.kind() != "object" || depth > 8 {
        return UpdateFlow::None;
    }
    let mut flow = UpdateFlow::None;
    let mut cursor = object.walk();
    for child in object.named_children(&mut cursor) {
        if child.kind() == "spread_element" {
            let Some(value) = child.named_child(0) else {
                continue;
            };
            if value.kind() == "identifier" && node_text(value, source) == updates_parameter {
                flow = UpdateFlow::Direct;
            }
            continue;
        }
        if child.kind() != "pair" {
            continue;
        }
        let matches_property = child
            .child_by_field_name("key")
            .and_then(|key| property_key(key, source))
            .as_deref()
            == Some(property);
        if !matches_property {
            continue;
        }
        flow = child
            .child_by_field_name("value")
            .map(|value| {
                expression_update_flow(
                    value,
                    helper,
                    updates_parameter,
                    property,
                    source,
                    depth + 1,
                )
            })
            .unwrap_or(UpdateFlow::None);
    }
    flow
}

fn expression_update_flow(
    expression: Node<'_>,
    helper: Node<'_>,
    updates_parameter: &str,
    property: &str,
    source: &str,
    depth: usize,
) -> UpdateFlow {
    if depth > 8 {
        return UpdateFlow::None;
    }
    if member_root_and_property(expression, source)
        .is_some_and(|(root, key)| root == updates_parameter && key == property)
    {
        return UpdateFlow::Direct;
    }
    if expression.kind() == "member_expression" {
        let Some(object) = expression.child_by_field_name("object") else {
            return UpdateFlow::None;
        };
        let Some(key) = expression.child_by_field_name("property") else {
            return UpdateFlow::None;
        };
        if object.kind() == "identifier" && node_text(key, source) == property {
            if let Some(initializer) = local_initializer(
                helper,
                node_text(object, source),
                expression.start_byte(),
                source,
            ) {
                return object_update_flow(
                    initializer,
                    helper,
                    updates_parameter,
                    property,
                    source,
                    depth + 1,
                );
            }
        }
    }
    if expression_uses_update_property(
        expression,
        helper,
        updates_parameter,
        property,
        source,
        depth + 1,
    ) {
        UpdateFlow::Complex
    } else {
        UpdateFlow::None
    }
}

fn callable_parameter_names(callable: Node<'_>, source: &str) -> Vec<String> {
    let Some(parameters) = callable.child_by_field_name("parameters") else {
        return callable
            .child_by_field_name("parameter")
            .filter(|parameter| parameter.kind() == "identifier")
            .map(|parameter| vec![node_text(parameter, source).to_string()])
            .unwrap_or_default();
    };
    let mut cursor = parameters.walk();
    parameters
        .named_children(&mut cursor)
        .filter(|parameter| parameter.kind() == "identifier")
        .map(|parameter| node_text(parameter, source).to_string())
        .collect()
}

fn is_provider_config_setter(function: Node<'_>, source: &str) -> bool {
    let parameters = callable_parameter_names(function, source);
    let Some(settings_parameter) = parameters.first() else {
        return false;
    };
    let mut found = false;
    collect_scope_nodes(function, &mut |node| {
        if found || node.kind() != "assignment_expression" {
            return;
        }
        let Some(left) = node.child_by_field_name("left") else {
            return;
        };
        found = member_root_and_property(left, source).is_some_and(|(root, property)| {
            root == *settings_parameter && property == "providerConfigs"
        });
    });
    found
}

fn member_root_and_property(node: Node<'_>, source: &str) -> Option<(String, String)> {
    if node.kind() != "member_expression" {
        return None;
    }
    let object = node.child_by_field_name("object")?;
    let property = node.child_by_field_name("property")?;
    (object.kind() == "identifier").then(|| {
        (
            node_text(object, source).to_string(),
            node_text(property, source).to_string(),
        )
    })
}

fn object_has_property(object: Node<'_>, expected: &str, source: &str) -> bool {
    if object.kind() != "object" {
        return false;
    }
    let mut cursor = object.walk();
    let found = object.named_children(&mut cursor).any(|child| {
        if child.kind() == "pair" {
            return child
                .child_by_field_name("key")
                .and_then(|key| property_key(key, source))
                .as_deref()
                == Some(expected);
        }
        matches!(
            child.kind(),
            "shorthand_property_identifier" | "shorthand_property_identifier_pattern"
        ) && node_text(child, source) == expected
    });
    found
}

fn expression_uses_update_property(
    expression: Node<'_>,
    helper: Node<'_>,
    updates_parameter: &str,
    property: &str,
    source: &str,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    if expression.kind() == "member_expression" {
        if member_root_and_property(expression, source)
            .is_some_and(|(root, key)| root == updates_parameter && key == property)
        {
            return true;
        }
        let Some(object) = expression.child_by_field_name("object") else {
            return false;
        };
        let Some(key) = expression.child_by_field_name("property") else {
            return false;
        };
        if object.kind() == "identifier" && node_text(key, source) == property {
            if let Some(initializer) = local_initializer(
                helper,
                node_text(object, source),
                expression.start_byte(),
                source,
            ) {
                return object_value_uses_update_property(
                    initializer,
                    helper,
                    updates_parameter,
                    property,
                    source,
                    depth + 1,
                );
            }
        }
    }
    if expression.kind() == "identifier" {
        if let Some(initializer) = local_initializer(
            helper,
            node_text(expression, source),
            expression.start_byte(),
            source,
        ) {
            return object_value_uses_update_property(
                initializer,
                helper,
                updates_parameter,
                property,
                source,
                depth + 1,
            );
        }
    }
    let mut found = false;
    collect_nodes(expression, &mut |node| {
        if found || node.kind() != "member_expression" {
            return;
        }
        found = member_root_and_property(node, source)
            .is_some_and(|(root, key)| root == updates_parameter && key == property);
    });
    found
}

fn object_value_uses_update_property(
    value: Node<'_>,
    helper: Node<'_>,
    updates_parameter: &str,
    property: &str,
    source: &str,
    depth: usize,
) -> bool {
    if value.kind() != "object" {
        return expression_uses_update_property(
            value,
            helper,
            updates_parameter,
            property,
            source,
            depth,
        );
    }
    let mut cursor = value.walk();
    for child in value.named_children(&mut cursor) {
        if child.kind() == "spread_element" {
            if child.named_child(0).is_some_and(|argument| {
                argument.kind() == "identifier" && node_text(argument, source) == updates_parameter
            }) {
                return true;
            }
            continue;
        }
        if child.kind() != "pair" {
            continue;
        }
        let matches_property = child
            .child_by_field_name("key")
            .and_then(|key| property_key(key, source))
            .as_deref()
            == Some(property);
        if matches_property
            && child
                .child_by_field_name("value")
                .is_some_and(|child_value| {
                    expression_uses_update_property(
                        child_value,
                        helper,
                        updates_parameter,
                        property,
                        source,
                        depth + 1,
                    )
                })
        {
            return true;
        }
    }
    false
}

fn dynamic_output_property(
    helper: Node<'_>,
    updates_parameter: &str,
    update_property: &str,
    config_object: Node<'_>,
    source: &str,
) -> Option<String> {
    let expected_update = format!("{updates_parameter}.{update_property}");
    let mut candidates = BTreeSet::new();
    collect_scope_nodes(helper, &mut |assignment| {
        if assignment.kind() != "assignment_expression" {
            return;
        }
        let Some(left) = assignment.child_by_field_name("left") else {
            return;
        };
        let Some(right) = assignment.child_by_field_name("right") else {
            return;
        };
        if !node_text(right, source).contains(&expected_update)
            || left.kind() != "subscript_expression"
        {
            return;
        }
        let Some(object) = left.child_by_field_name("object") else {
            return;
        };
        if object.kind() == "identifier" {
            candidates.insert(node_text(object, source).to_string());
        }
    });
    candidates
        .into_iter()
        .find(|candidate| object_has_property(config_object, candidate, source))
}

fn getter_read_paths(
    getter_name: &str,
    property: &str,
    canonical_path: &str,
    source: &str,
    constants: &HashMap<String, Value>,
    program_index: &SettingsProgramIndex<'_>,
) -> Vec<String> {
    let Some(getter) = program_index.functions.get(getter_name).copied() else {
        return vec![canonical_path.to_string()];
    };
    let parameters = callable_parameter_names(getter, source);
    let Some(settings_parameter) = parameters.first() else {
        return vec![canonical_path.to_string()];
    };
    let mut provider_bindings = BTreeMap::<String, String>::new();
    collect_scope_nodes(getter, &mut |declarator| {
        if declarator.kind() != "variable_declarator" {
            return;
        }
        let Some(name) = declarator.child_by_field_name("name") else {
            return;
        };
        let Some(call) = declarator.child_by_field_name("value") else {
            return;
        };
        if name.kind() != "identifier" || call.kind() != "call_expression" {
            return;
        }
        let Some(function_name) = call_function_identifier(call, source) else {
            return;
        };
        let Some(function) = program_index.functions.get(&function_name).copied() else {
            return;
        };
        if !is_provider_config_getter(function, source) {
            return;
        }
        let arguments = call_arguments(call);
        if arguments.first().is_none_or(|argument| {
            argument.kind() != "identifier" || node_text(*argument, source) != settings_parameter
        }) {
            return;
        }
        let Some(provider_id) = arguments
            .get(1)
            .and_then(|argument| static_value(*argument, source, constants, 0))
            .and_then(value_to_string)
        else {
            return;
        };
        provider_bindings.insert(node_text(name, source).to_string(), provider_id);
    });

    let Some(value) = returned_object_property(getter, property, source) else {
        return vec![canonical_path.to_string()];
    };
    let mut paths = vec![canonical_path.to_string()];
    collect_nodes(value, &mut |member| {
        let Some((root, key)) = member_root_and_property(member, source) else {
            return;
        };
        let candidate = if root == *settings_parameter {
            Some(pointer_for_key(&key))
        } else {
            provider_bindings.get(&root).map(|provider_id| {
                pointer_from_segments(&["providerConfigs".to_string(), provider_id.clone(), key])
            })
        };
        if let Some(candidate) = candidate {
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    });
    paths
}

fn is_provider_config_getter(function: Node<'_>, source: &str) -> bool {
    let parameters = callable_parameter_names(function, source);
    let (Some(settings_parameter), Some(provider_parameter)) =
        (parameters.first(), parameters.get(1))
    else {
        return false;
    };
    let mut reads_root = false;
    let mut indexes_provider = false;
    collect_scope_nodes(function, &mut |node| match node.kind() {
        "member_expression" => {
            reads_root |= member_root_and_property(node, source).is_some_and(|(root, property)| {
                root == *settings_parameter && property == "providerConfigs"
            });
        }
        "subscript_expression" => {
            let index = node.child_by_field_name("index");
            indexes_provider |= index.is_some_and(|index| {
                index.kind() == "identifier" && node_text(index, source) == provider_parameter
            });
        }
        _ => {}
    });
    reads_root && indexes_provider
}

fn returned_object_property<'tree>(
    function: Node<'tree>,
    expected: &str,
    source: &str,
) -> Option<Node<'tree>> {
    let mut found = None;
    collect_scope_nodes(function, &mut |return_statement| {
        if found.is_some() || return_statement.kind() != "return_statement" {
            return;
        }
        let Some(object) = return_statement.named_child(0) else {
            return;
        };
        if object.kind() != "object" {
            return;
        }
        let mut cursor = object.walk();
        found = object.named_children(&mut cursor).find_map(|child| {
            if child.kind() != "pair" {
                return None;
            }
            let key = child
                .child_by_field_name("key")
                .and_then(|key| property_key(key, source))?;
            (key == expected)
                .then(|| child.child_by_field_name("value"))
                .flatten()
        });
    });
    found
}

fn enumerate_path_options(
    configuration: Option<&Value>,
    prefix: &str,
) -> Vec<PluginSettingPathOption> {
    let Some(object) = configuration
        .and_then(|value| value.pointer(prefix))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    object
        .keys()
        .map(|key| PluginSettingPathOption {
            path: format!("{prefix}/{}", escape_pointer_segment(key)),
            label: abbreviated_dynamic_key(key),
            detail: key.clone(),
        })
        .collect()
}

fn abbreviated_dynamic_key(key: &str) -> String {
    if key.chars().count() <= 28 {
        return key.to_string();
    }
    let start = key.chars().take(12).collect::<String>();
    let end = key
        .chars()
        .rev()
        .take(10)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{start}...{end}")
}

fn imperative_field_from_statement(
    statement: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    configuration: Option<&Value>,
    program_index: &SettingsProgramIndex<'_>,
    index: usize,
) -> Option<ExtractedField> {
    let mut calls = Vec::new();
    collect_nodes(statement, &mut |node| {
        if node.kind() == "call_expression" {
            calls.push(node);
        }
    });
    if let Some((binding, scope)) = bound_setting_scope(statement, source) {
        collect_scope_nodes(scope, &mut |call| {
            if call.kind() != "call_expression"
                || call_receiver_identifier(call, source).as_deref() != Some(binding.as_str())
            {
                return;
            }
            collect_nodes(call, &mut |node| {
                if node.kind() == "call_expression" {
                    calls.push(node);
                }
            });
        });
    }
    calls.sort_by_key(Node::start_byte);

    let name_value = first_call_value(&calls, "setName", source, constants);
    let name_unresolved = name_value.as_ref().is_some_and(is_unresolved_i18n_value);
    let name = name_value.and_then(value_to_string);
    let description_value = first_call_value(&calls, "setDesc", source, constants);
    let description_unresolved = description_value
        .as_ref()
        .is_some_and(is_unresolved_i18n_value);
    let description = description_value.and_then(value_to_string);
    let heading = calls
        .iter()
        .any(|call| call_method_name(*call, source).as_deref() == Some("setHeading"));
    if heading {
        let heading_name = name?;
        let mut field = new_field(
            format!("imperative-heading-{index}"),
            None,
            heading_name,
            description,
            PluginSettingControl::Heading,
            PluginSettingSource::Imperative,
            PluginSettingConfidence::Exact,
        );
        if name_unresolved || description_unresolved {
            field
                .warnings
                .push("翻译标签未能静态解析，已显示稳定键名".to_string());
        }
        return Some(ExtractedField {
            field,
            page_path: Vec::new(),
        });
    }

    let control_call = calls
        .iter()
        .filter_map(|call| {
            let method = call_method_name(*call, source)?;
            let control = control_from_imperative_method(&method)?;
            Some((*call, control, method))
        })
        .min_by_key(|(call, _, method)| {
            (
                usize::from(matches!(
                    method.as_str(),
                    "addButton" | "addExtraButton" | "addComponent"
                )),
                call.start_byte(),
            )
        });
    let (control_call, mut control, control_method) = control_call?;
    let mut control_calls = Vec::new();
    collect_nodes(control_call, &mut |node| {
        if node.kind() == "call_expression" {
            control_calls.push(node);
        }
    });
    control_calls.sort_by_key(Node::start_byte);

    if matches!(control, PluginSettingControl::Text)
        && (control_calls.iter().any(|call| {
            call_method_name(*call, source).as_deref() == Some("setInputType")
                && call_arguments(*call)
                    .first()
                    .and_then(|argument| static_value(*argument, source, constants, 0))
                    .and_then(value_to_string)
                    .as_deref()
                    == Some("password")
        }) || assignment_sets_password_input_type(control_call, source, constants))
    {
        control = PluginSettingControl::Password;
    }

    let mut read_candidates = BTreeSet::new();
    for call in &control_calls {
        if call_method_name(*call, source).as_deref() == Some("setValue") {
            if let Some(argument) = call_arguments(*call).first() {
                read_candidates.extend(pointer_candidates_from_expression(
                    *argument, source, constants,
                ));
            }
        }
    }

    let mut write_candidates = BTreeSet::new();
    let mut has_complex_write = false;
    for call in &control_calls {
        if call_method_name(*call, source).as_deref() != Some("onChange") {
            continue;
        }
        let Some(callback) = call_arguments(*call).first().copied() else {
            continue;
        };
        let callback_parameters = callback_parameter_names(callback, source);
        collect_nodes(callback, &mut |node| {
            if node.kind() != "assignment_expression" {
                return;
            }
            let Some(left) = node.child_by_field_name("left") else {
                return;
            };
            if let Some(pointer) = pointer_from_expression(left, source, constants) {
                write_candidates.insert(pointer);
                if !assignment_directly_uses_parameter(node, source, &callback_parameters) {
                    has_complex_write = true;
                }
            }
        });
    }

    let indirect = infer_indirect_path_binding(
        statement,
        &control_calls,
        source,
        constants,
        configuration,
        program_index,
    );
    if let Some(path) = indirect.path.as_ref() {
        write_candidates.insert(path.clone());
        read_candidates.extend(indirect.read_paths.iter().cloned());
    }
    has_complex_write |= indirect.complex_write;

    let mut field_warnings = Vec::new();
    if name_unresolved || description_unresolved {
        field_warnings.push("翻译标签未能静态解析，已显示稳定键名".to_string());
    }
    let canonical_write_path = (write_candidates.len() == 1)
        .then(|| write_candidates.iter().next().cloned())
        .flatten();
    let path = if canonical_write_path
        .as_ref()
        .is_some_and(|path| read_candidates.contains(path))
    {
        canonical_write_path
    } else if read_candidates.len() == 1 {
        read_candidates.iter().next().cloned()
    } else if !indirect.path_options.is_empty() {
        None
    } else {
        if read_candidates.len() > 1 {
            field_warnings.push("检测到多个读取路径，无法确定当前值".to_string());
        } else {
            field_warnings.push("无法确定该设置从 data.json 读取的位置".to_string());
        }
        None
    };
    let read_only = if !indirect.path_options.is_empty() {
        has_complex_write
    } else {
        match path.as_ref() {
            Some(read_path)
                if write_candidates.len() == 1
                    && write_candidates.contains(read_path)
                    && !has_complex_write =>
            {
                false
            }
            Some(_) => {
                if write_candidates.is_empty() {
                    field_warnings.push("未找到可验证的 onChange 直接写回，已设为只读".to_string());
                } else if write_candidates.len() > 1 {
                    field_warnings.push("该控件会写入多个配置路径，已设为只读".to_string());
                } else if !write_candidates
                    .iter()
                    .all(|candidate| path.as_ref() == Some(candidate))
                {
                    field_warnings.push("读取路径与写入路径不一致，已设为只读".to_string());
                }
                if has_complex_write {
                    field_warnings.push("写入包含值转换或复合赋值，已设为只读".to_string());
                }
                true
            }
            None => true,
        }
    };
    if read_only && name.is_none() && description.is_none() {
        return None;
    }
    let fallback_name = path
        .as_deref()
        .and_then(pointer_last_segment)
        .map(|key| humanize_key(&key))
        .unwrap_or_else(|| "自定义设置".to_string());
    let mut field = new_field(
        format!(
            "imperative-{}",
            path.as_deref().unwrap_or(&index.to_string())
        ),
        path,
        name.unwrap_or(fallback_name),
        description,
        control,
        PluginSettingSource::Imperative,
        if read_only {
            PluginSettingConfidence::Inferred
        } else {
            PluginSettingConfidence::Exact
        },
    );
    field.read_only = read_only;
    if !indirect.read_paths.is_empty() {
        field.read_paths = indirect.read_paths;
    }
    field.path_options = indirect.path_options;
    field.warnings = field_warnings;
    if indirect.dynamic_path {
        field.warnings.push(if field.path_options.is_empty() {
            "该设置使用运行时动态键，data.json 中没有可选择的已有配置键".to_string()
        } else {
            "该设置使用运行时动态键，请明确选择一个已有配置键".to_string()
        });
    }
    field.placeholder = first_call_value(&control_calls, "setPlaceholder", source, constants)
        .and_then(value_to_string);
    field.default_value = first_call_value(&control_calls, "setValue", source, constants)
        .filter(|value| !value.is_object() && !value.is_array());
    if let Some(disabled_call) = control_calls
        .iter()
        .find(|call| call_method_name(**call, source).as_deref() == Some("setDisabled"))
    {
        let disabled = call_arguments(*disabled_call)
            .first()
            .and_then(|argument| static_value(*argument, source, constants, 0))
            .and_then(|value| value.as_bool());
        if disabled != Some(false) {
            field.read_only = true;
            field.warnings.push(if disabled == Some(true) {
                "插件将该控件设为禁用，已保持只读".to_string()
            } else {
                "无法静态确定控件是否禁用，已保持只读".to_string()
            });
        }
    }
    populate_imperative_options_and_limits(
        &mut field,
        &control_calls,
        source,
        constants,
        program_index,
    );
    field.support = classify_field_support(
        &field,
        matches!(control_method.as_str(), "addButton" | "addExtraButton"),
    );
    Some(ExtractedField {
        field,
        page_path: Vec::new(),
    })
}

fn bound_setting_scope<'tree>(
    statement: Node<'tree>,
    source: &str,
) -> Option<(String, Node<'tree>)> {
    let parent = statement.parent()?;
    if parent.kind() != "variable_declarator"
        || parent.child_by_field_name("value") != Some(statement)
    {
        return None;
    }
    let name = parent.child_by_field_name("name")?;
    if name.kind() != "identifier" {
        return None;
    }
    Some((
        node_text(name, source).to_string(),
        enclosing_callable(parent)?,
    ))
}

fn assignment_sets_password_input_type(
    scope: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
) -> bool {
    let mut found = false;
    collect_nodes(scope, &mut |node| {
        if found || node.kind() != "assignment_expression" {
            return;
        }
        let Some(left) = node.child_by_field_name("left") else {
            return;
        };
        let Some(right) = node.child_by_field_name("right") else {
            return;
        };
        found = node_text(left, source).ends_with(".inputEl.type")
            && static_value(right, source, constants, 0)
                .and_then(value_to_string)
                .as_deref()
                == Some("password");
    });
    found
}

fn callback_parameter_names(callback: Node<'_>, source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    if let Some(parameter) = callback.child_by_field_name("parameter") {
        if parameter.kind() == "identifier" {
            names.insert(node_text(parameter, source).to_string());
        }
    }
    if let Some(parameters) = callback.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        for parameter in parameters.named_children(&mut cursor) {
            if parameter.kind() == "identifier" {
                names.insert(node_text(parameter, source).to_string());
            }
        }
    }
    names
}

fn assignment_directly_uses_parameter(
    assignment: Node<'_>,
    source: &str,
    parameters: &BTreeSet<String>,
) -> bool {
    let Some(left) = assignment.child_by_field_name("left") else {
        return false;
    };
    let Some(mut right) = assignment.child_by_field_name("right") else {
        return false;
    };
    let operator = source
        .get(left.end_byte()..right.start_byte())
        .unwrap_or_default()
        .trim();
    if operator != "=" {
        return false;
    }
    while right.kind() == "parenthesized_expression" {
        let Some(child) = right.named_child(0) else {
            return false;
        };
        right = child;
    }
    right.kind() == "identifier" && parameters.contains(node_text(right, source))
}

fn populate_imperative_options_and_limits(
    field: &mut PluginSettingField,
    calls: &[Node<'_>],
    source: &str,
    constants: &HashMap<String, Value>,
    program_index: &SettingsProgramIndex<'_>,
) {
    let mut unresolved_options = false;
    for call in calls {
        match call_method_name(*call, source).as_deref() {
            Some("addOption") => {
                let options = static_add_option_values(*call, source, constants, program_index);
                unresolved_options |= options.is_empty();
                for (value, label) in options {
                    field.options.push(PluginSettingOption {
                        label: label.unwrap_or_else(|| display_json_value(&value)),
                        value,
                    });
                }
            }
            Some("addOptions") => {
                if let Some(value) = call_arguments(*call).first().and_then(|argument| {
                    static_control_value(
                        *argument,
                        source,
                        constants,
                        program_index,
                        0,
                        &mut BTreeSet::new(),
                    )
                }) {
                    field.options.extend(options_from_value(&value));
                } else {
                    unresolved_options = true;
                }
            }
            Some("setLimits") => {
                let arguments = call_arguments(*call);
                let mut call_stack = BTreeSet::new();
                field.min = arguments.first().and_then(|argument| {
                    static_control_value(
                        *argument,
                        source,
                        constants,
                        program_index,
                        0,
                        &mut call_stack,
                    )
                    .and_then(|value| value.as_f64())
                });
                field.max = arguments.get(1).and_then(|argument| {
                    static_control_value(
                        *argument,
                        source,
                        constants,
                        program_index,
                        0,
                        &mut call_stack,
                    )
                    .and_then(|value| value.as_f64())
                });
                field.step = arguments.get(2).and_then(|argument| {
                    static_control_value(
                        *argument,
                        source,
                        constants,
                        program_index,
                        0,
                        &mut call_stack,
                    )
                    .and_then(|value| value.as_f64())
                });
            }
            _ => {}
        }
    }
    deduplicate_options(&mut field.options);
    if matches!(field.control, PluginSettingControl::Dropdown)
        && (field.options.is_empty() || unresolved_options)
    {
        field.warnings.push(
            "下拉选项包含无法静态解析的运行时来源，已保留下拉控件且仅展示可确认的选项".to_string(),
        );
    }
}

fn static_add_option_values(
    call: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    program_index: &SettingsProgramIndex<'_>,
) -> Vec<(Value, Option<String>)> {
    let arguments = call_arguments(call);
    let direct_value = arguments.first().and_then(|argument| {
        static_control_value(
            *argument,
            source,
            constants,
            program_index,
            0,
            &mut BTreeSet::new(),
        )
    });
    if let Some(value) = direct_value {
        let label = arguments.get(1).and_then(|argument| {
            static_control_value(
                *argument,
                source,
                constants,
                program_index,
                0,
                &mut BTreeSet::new(),
            )
            .and_then(value_to_string)
        });
        return vec![(value, label)];
    }

    let Some((binding, values)) = static_for_of_binding(call, source, constants, program_index)
    else {
        return Vec::new();
    };
    values
        .into_iter()
        .filter_map(|item| {
            let mut loop_constants = constants.clone();
            loop_constants.insert(binding.clone(), item);
            let value = arguments.first().and_then(|argument| {
                static_control_value(
                    *argument,
                    source,
                    &loop_constants,
                    program_index,
                    0,
                    &mut BTreeSet::new(),
                )
            })?;
            let label = arguments.get(1).and_then(|argument| {
                static_control_value(
                    *argument,
                    source,
                    &loop_constants,
                    program_index,
                    0,
                    &mut BTreeSet::new(),
                )
                .and_then(value_to_string)
            });
            Some((value, label))
        })
        .collect()
}

fn static_for_of_binding(
    mut node: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    program_index: &SettingsProgramIndex<'_>,
) -> Option<(String, Vec<Value>)> {
    while let Some(parent) = node.parent() {
        if parent.kind() == "for_in_statement" {
            let statement_text = node_text(parent, source);
            if !statement_text.trim_start().starts_with("for") || !statement_text.contains(" of ") {
                return None;
            }
            let left = parent.child_by_field_name("left")?;
            let right = parent.child_by_field_name("right")?;
            let binding = loop_binding_name(left, source)?;
            let values = static_control_value(
                right,
                source,
                constants,
                program_index,
                0,
                &mut BTreeSet::new(),
            )?
            .as_array()?
            .clone();
            return Some((binding, values));
        }
        if matches!(
            parent.kind(),
            "method_definition" | "function_declaration" | "function_expression" | "arrow_function"
        ) {
            return None;
        }
        node = parent;
    }
    None
}

fn deduplicate_options(options: &mut Vec<PluginSettingOption>) {
    let mut seen = BTreeSet::new();
    options.retain(|option| {
        serde_json::to_string(&option.value)
            .ok()
            .is_some_and(|value| seen.insert(value))
    });
}

fn loop_binding_name(left: Node<'_>, source: &str) -> Option<String> {
    if left.kind() == "identifier" {
        return Some(node_text(left, source).to_string());
    }
    let mut binding = None;
    collect_nodes(left, &mut |node| {
        if binding.is_some() || node.kind() != "variable_declarator" {
            return;
        }
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        if name.kind() == "identifier" {
            binding = Some(node_text(name, source).to_string());
        }
    });
    binding
}

fn build_groups(fields: Vec<ExtractedField>) -> Vec<PluginSettingGroup> {
    let mut groups = Vec::new();
    let mut current_title: Option<String> = None;
    let mut current_page = Vec::new();
    let mut current_fields = Vec::new();
    let mut group_index = 0usize;

    let flush = |groups: &mut Vec<PluginSettingGroup>,
                 fields: &mut Vec<PluginSettingField>,
                 title: &Option<String>,
                 page: &[String],
                 index: &mut usize| {
        if fields.is_empty() {
            return;
        }
        groups.push(PluginSettingGroup {
            id: format!("inferred-group-{index}"),
            title: title.clone(),
            page_path: page.to_vec(),
            fields: std::mem::take(fields),
        });
        *index += 1;
    };

    for item in fields {
        if item.page_path != current_page {
            flush(
                &mut groups,
                &mut current_fields,
                &current_title,
                &current_page,
                &mut group_index,
            );
            current_page = item.page_path.clone();
            current_title = None;
        }
        if matches!(item.field.control, PluginSettingControl::Heading) {
            if current_title.as_deref() == Some(item.field.name.as_str()) {
                continue;
            }
            flush(
                &mut groups,
                &mut current_fields,
                &current_title,
                &current_page,
                &mut group_index,
            );
            current_title = Some(item.field.name);
        } else {
            current_fields.push(item.field);
        }
    }
    flush(
        &mut groups,
        &mut current_fields,
        &current_title,
        &current_page,
        &mut group_index,
    );
    groups
}

fn new_field(
    id: String,
    path: Option<String>,
    name: String,
    description: Option<String>,
    control: PluginSettingControl,
    source: PluginSettingSource,
    confidence: PluginSettingConfidence,
) -> PluginSettingField {
    PluginSettingField {
        id,
        read_paths: path.iter().cloned().collect(),
        path,
        path_options: Vec::new(),
        name,
        description,
        control,
        options: Vec::new(),
        placeholder: None,
        min: None,
        max: None,
        step: None,
        default_value: None,
        source,
        confidence,
        support: PluginSettingSupport::UnresolvedRuntime,
        read_only: false,
        warnings: Vec::new(),
    }
}

fn classify_field_support(field: &PluginSettingField, action_only: bool) -> PluginSettingSupport {
    if action_only {
        return PluginSettingSupport::ActionOnly;
    }
    if !field.path_options.is_empty() {
        return PluginSettingSupport::DynamicExistingKey;
    }
    if field.path.is_some() {
        return if field.read_only {
            PluginSettingSupport::RiskTransform
        } else {
            PluginSettingSupport::SafeWritable
        };
    }
    if matches!(field.control, PluginSettingControl::Unsupported) {
        PluginSettingSupport::UnsupportedCustom
    } else {
        PluginSettingSupport::UnresolvedRuntime
    }
}

fn settings_coverage(groups: &[PluginSettingGroup]) -> PluginSettingsCoverage {
    let mut coverage = PluginSettingsCoverage::default();
    for field in groups.iter().flat_map(|group| &group.fields) {
        coverage.total += 1;
        match field.support {
            PluginSettingSupport::SafeWritable => coverage.safe_writable += 1,
            PluginSettingSupport::RiskTransform => coverage.risk_transform += 1,
            PluginSettingSupport::DynamicExistingKey => coverage.dynamic_existing_key += 1,
            PluginSettingSupport::ActionOnly => coverage.action_only += 1,
            PluginSettingSupport::UnresolvedRuntime => coverage.unresolved_runtime += 1,
            PluginSettingSupport::UnsupportedCustom => coverage.unsupported_custom += 1,
        }
    }
    coverage
}

pub fn merge_runtime_settings_presentation(
    schema: &mut PluginSettingsSchema,
    snapshot: &PluginRuntimeSettingsSnapshot,
) -> usize {
    let mut merged = 0usize;
    let mut appended = 0usize;
    for runtime in snapshot.fields.iter().filter(|field| field.visible) {
        let mut candidates = Vec::new();
        for (group_index, group) in schema.groups.iter().enumerate() {
            if group.page_path != runtime.page_path
                || runtime.group_title.as_ref().is_some_and(|title| {
                    group
                        .title
                        .as_ref()
                        .is_none_or(|group_title| !same_setting_label(group_title, title))
                })
            {
                continue;
            }
            for (field_index, field) in group.fields.iter().enumerate() {
                if same_setting_label(&field.name, &runtime.name)
                    && runtime_control_compatible(&field.control, &runtime.control)
                {
                    candidates.push((group_index, field_index));
                }
            }
        }

        if candidates.len() > 1 {
            let ordered = candidates
                .iter()
                .copied()
                .filter(|(_, field_index)| *field_index == runtime.order)
                .collect::<Vec<_>>();
            if ordered.len() == 1 {
                candidates = ordered;
            }
        }
        let Some((group_index, field_index)) = (candidates.len() == 1).then(|| candidates[0])
        else {
            let conflict = candidates.len() > 1;
            if conflict {
                schema.warnings.push(format!(
                    "运行时快照中的设置“{}”匹配到多个静态字段，已保留为独立只读行",
                    runtime.name
                ));
            }
            append_runtime_only_field(schema, runtime, conflict);
            appended += 1;
            continue;
        };

        let field = &mut schema.groups[group_index].fields[field_index];
        if let Some(description) = runtime.description.as_ref() {
            field.description = Some(description.clone());
        }
        if !runtime.action && !matches!(runtime.control, PluginSettingControl::Unsupported) {
            field.control = runtime.control.clone();
        }
        if !runtime.options.is_empty() {
            field.options = runtime.options.clone();
        }
        if let Some(placeholder) = runtime.placeholder.as_ref() {
            field.placeholder = Some(placeholder.clone());
        }
        field.min = runtime.min.or(field.min);
        field.max = runtime.max.or(field.max);
        field.step = runtime.step.or(field.step);
        if runtime.disabled {
            field
                .warnings
                .push("Obsidian 运行时将该控件标记为禁用".to_string());
        }
        merged += 1;
    }
    if merged > 0 {
        schema.warnings.push(format!(
            "已合并 {merged} 项 Obsidian 运行时显示元数据；写入权限仍以静态分析为准"
        ));
    }
    if appended > 0 {
        schema.warnings.push(format!(
            "已显示 {appended} 项仅由 Bridge 观察到的运行时控件；这些控件没有写入路径"
        ));
    }
    for warning in &snapshot.warnings {
        schema.warnings.push(format!("Bridge：{warning}"));
    }
    if appended > 0 {
        schema.coverage = settings_coverage(&schema.groups);
    }
    merged
}

fn append_runtime_only_field(
    schema: &mut PluginSettingsSchema,
    runtime: &PluginRuntimeSettingField,
    conflict: bool,
) {
    let group_index = schema
        .groups
        .iter()
        .position(|group| {
            group.page_path == runtime.page_path
                && match (&group.title, &runtime.group_title) {
                    (Some(left), Some(right)) => same_setting_label(left, right),
                    (None, None) => true,
                    _ => false,
                }
        })
        .unwrap_or_else(|| {
            let index = schema.groups.len();
            schema.groups.push(PluginSettingGroup {
                id: format!("bridge-runtime-group-{index}"),
                title: runtime.group_title.clone(),
                page_path: runtime.page_path.clone(),
                fields: Vec::new(),
            });
            index
        });
    let support = if runtime.action {
        PluginSettingSupport::ActionOnly
    } else if matches!(runtime.control, PluginSettingControl::Unsupported) {
        PluginSettingSupport::UnsupportedCustom
    } else {
        PluginSettingSupport::UnresolvedRuntime
    };
    let mut warnings =
        vec!["仅由 Obsidian Bridge 观察到显示结构，未证明 data.json 写入路径".to_string()];
    if conflict {
        warnings.push("名称和控件类型匹配到多个静态字段，无法安全关联".to_string());
    }
    if runtime.disabled {
        warnings.push("Obsidian 运行时将该控件标记为禁用".to_string());
    }
    let field_index = schema.groups[group_index].fields.len();
    schema.groups[group_index].fields.push(PluginSettingField {
        id: format!("bridge-runtime-{}-{field_index}", runtime.order),
        path: None,
        read_paths: Vec::new(),
        path_options: Vec::new(),
        name: runtime.name.clone(),
        description: runtime.description.clone(),
        control: runtime.control.clone(),
        options: runtime.options.clone(),
        placeholder: runtime.placeholder.clone(),
        min: runtime.min,
        max: runtime.max,
        step: runtime.step,
        default_value: None,
        source: PluginSettingSource::Imperative,
        confidence: runtime.confidence.clone(),
        support,
        read_only: true,
        warnings,
    });
}

fn same_setting_label(left: &str, right: &str) -> bool {
    left.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .eq_ignore_ascii_case(&right.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn runtime_control_compatible(
    static_control: &PluginSettingControl,
    runtime_control: &PluginSettingControl,
) -> bool {
    static_control == runtime_control
        || matches!(static_control, PluginSettingControl::Unsupported)
        || matches!(
            (static_control, runtime_control),
            (PluginSettingControl::Text, PluginSettingControl::Password)
                | (PluginSettingControl::Password, PluginSettingControl::Text)
        )
}

fn control_from_declarative_type(value: &str) -> PluginSettingControl {
    match value.to_ascii_lowercase().as_str() {
        "toggle" | "boolean" => PluginSettingControl::Toggle,
        "text" | "string" => PluginSettingControl::Text,
        "textarea" | "multiline" => PluginSettingControl::Textarea,
        "dropdown" | "select" => PluginSettingControl::Dropdown,
        "slider" => PluginSettingControl::Slider,
        "number" => PluginSettingControl::Number,
        "color" | "color-picker" => PluginSettingControl::Color,
        "password" | "secret" => PluginSettingControl::Password,
        _ => PluginSettingControl::Unsupported,
    }
}

fn control_from_imperative_method(method: &str) -> Option<PluginSettingControl> {
    match method {
        "addToggle" => Some(PluginSettingControl::Toggle),
        "addText" | "addSearch" => Some(PluginSettingControl::Text),
        "addTextArea" => Some(PluginSettingControl::Textarea),
        "addDropdown" => Some(PluginSettingControl::Dropdown),
        "addSlider" => Some(PluginSettingControl::Slider),
        "addColorPicker" => Some(PluginSettingControl::Color),
        "addButton" | "addExtraButton" | "addComponent" => Some(PluginSettingControl::Unsupported),
        _ => None,
    }
}

fn options_from_value(value: &Value) -> Vec<PluginSettingOption> {
    match value {
        Value::Object(object) => object
            .iter()
            .map(|(option_value, label)| PluginSettingOption {
                value: Value::String(option_value.clone()),
                label: label
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| display_json_value(label)),
            })
            .collect(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                if let Some(object) = item.as_object() {
                    let option_value = object.get("value")?.clone();
                    let label = object
                        .get("label")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| display_json_value(&option_value));
                    Some(PluginSettingOption {
                        value: option_value,
                        label,
                    })
                } else {
                    Some(PluginSettingOption {
                        value: item.clone(),
                        label: display_json_value(item),
                    })
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn object_property_value(
    object: Node<'_>,
    key: &str,
    source: &str,
    constants: &HashMap<String, Value>,
) -> Option<Value> {
    object_property_node(object, key, source)
        .and_then(|node| static_value(node, source, constants, 0))
}

fn object_property_number(
    object: Node<'_>,
    key: &str,
    source: &str,
    constants: &HashMap<String, Value>,
) -> Option<f64> {
    object_property_value(object, key, source, constants).and_then(|value| value.as_f64())
}

fn object_property_node<'tree>(
    object: Node<'tree>,
    expected_key: &str,
    source: &str,
) -> Option<Node<'tree>> {
    let mut cursor = object.walk();
    let value = object.named_children(&mut cursor).find_map(|child| {
        if child.kind() != "pair" {
            return None;
        }
        let key = child.child_by_field_name("key")?;
        if property_key(key, source).as_deref() != Some(expected_key) {
            return None;
        }
        child.child_by_field_name("value")
    });
    value
}

fn property_key(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "property_identifier" | "identifier" => Some(node_text(node, source).to_string()),
        "string" => parse_js_string(node_text(node, source)),
        _ => None,
    }
}

fn collect_static_constants(root: Node<'_>, source: &str) -> HashMap<String, Value> {
    let mut declarations = Vec::new();
    let mut counts = HashMap::<String, usize>::new();
    collect_nodes(root, &mut |node| {
        if node.kind() != "variable_declarator" {
            return;
        }
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Some(value_node) = node.child_by_field_name("value") else {
            return;
        };
        if name_node.kind() != "identifier" {
            return;
        }
        let name = node_text(name_node, source).to_string();
        *counts.entry(name.clone()).or_default() += 1;
        declarations.push((name, value_node));
    });

    let mut constants = HashMap::new();
    for _ in 0..4 {
        let mut changed = false;
        for (name, value_node) in &declarations {
            if counts.get(name) != Some(&1) || constants.contains_key(name) {
                continue;
            }
            if let Some(value) = static_value(*value_node, source, &constants, 0) {
                constants.insert(name.clone(), value);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    constants
}

fn collect_translation_constants(
    root: Node<'_>,
    source: &str,
    configuration: Option<&Value>,
    constants: &mut HashMap<String, Value>,
) {
    let namespaces = collect_export_namespaces(root, source, constants);
    let configured_locale = configuration
        .and_then(|value| value.pointer("/locale"))
        .and_then(Value::as_str);
    let default_locale = constants
        .iter()
        .find(|(name, value)| {
            name.to_ascii_lowercase().contains("default")
                && name.to_ascii_lowercase().contains("locale")
                && value.is_string()
        })
        .and_then(|(_, value)| value.as_str());
    let locale = configured_locale.or(default_locale).unwrap_or("en");

    let mut translation_roots = Vec::new();
    collect_nodes(root, &mut |node| {
        if node.kind() != "variable_declarator" {
            return;
        }
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let Some(value) = node.child_by_field_name("value") else {
            return;
        };
        if name.kind() != "identifier"
            || value.kind() != "object"
            || !node_text(name, source)
                .to_ascii_lowercase()
                .contains("translation")
        {
            return;
        }
        let Some(locale_value) = object_property_expression(value, locale, source) else {
            return;
        };
        let root_value = if locale_value.kind() == "identifier" {
            let binding = node_text(locale_value, source);
            namespaces
                .get(binding)
                .cloned()
                .or_else(|| constants.get(binding).cloned())
        } else {
            static_value(locale_value, source, constants, 0)
        };
        if let Some(root_value) = root_value {
            translation_roots.push(root_value);
        }
    });

    if translation_roots.len() != 1 {
        return;
    }
    let mut flattened = Vec::new();
    flatten_translation_values(&translation_roots[0], "", &mut flattened);
    for (key, value) in flattened {
        constants.insert(format!("{I18N_CONSTANT_PREFIX}{key}"), Value::String(value));
    }
}

fn collect_export_namespaces(
    root: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
) -> HashMap<String, Value> {
    let mut namespaces = HashMap::new();
    collect_nodes(root, &mut |call| {
        if call.kind() != "call_expression"
            || call_method_name(call, source).as_deref() != Some("__export")
        {
            return;
        }
        let arguments = call_arguments(call);
        let Some(target) = arguments.first().copied() else {
            return;
        };
        let Some(definitions) = arguments.get(1).copied() else {
            return;
        };
        if target.kind() != "identifier" || definitions.kind() != "object" {
            return;
        }
        let mut namespace = Map::new();
        let mut cursor = definitions.walk();
        for child in definitions.named_children(&mut cursor) {
            if child.kind() != "pair" {
                continue;
            }
            let Some(key) = child
                .child_by_field_name("key")
                .and_then(|key| property_key(key, source))
            else {
                continue;
            };
            let Some(getter) = child.child_by_field_name("value") else {
                continue;
            };
            let Some(body) = getter.child_by_field_name("body") else {
                continue;
            };
            let value = if body.kind() == "identifier" {
                constants.get(node_text(body, source)).cloned()
            } else {
                static_value(body, source, constants, 0)
            };
            if let Some(value) = value {
                namespace.insert(key, value);
            }
        }
        if !namespace.is_empty() {
            namespaces.insert(
                node_text(target, source).to_string(),
                Value::Object(namespace),
            );
        }
    });
    namespaces
}

fn flatten_translation_values(value: &Value, prefix: &str, output: &mut Vec<(String, String)>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_translation_values(child, &path, output);
            }
        }
        Value::String(value) if !prefix.is_empty() => {
            output.push((prefix.to_string(), value.clone()));
        }
        _ => {}
    }
}

fn static_value(
    node: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    depth: usize,
) -> Option<Value> {
    if depth > 20 {
        return None;
    }
    match node.kind() {
        "string" => parse_js_string(node_text(node, source)).map(Value::String),
        "template_string" => {
            let mut cursor = node.walk();
            if node
                .named_children(&mut cursor)
                .any(|child| child.kind() == "template_substitution")
            {
                None
            } else {
                let text = node_text(node, source);
                parse_js_string(text).map(Value::String)
            }
        }
        "number" => node_text(node, source)
            .replace('_', "")
            .parse::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .map(Value::Number),
        "true" => Some(Value::Bool(true)),
        "false" => Some(Value::Bool(false)),
        "null" => Some(Value::Null),
        "identifier" => constants.get(node_text(node, source)).cloned(),
        "member_expression" => {
            let object = static_value(
                node.child_by_field_name("object")?,
                source,
                constants,
                depth + 1,
            )?;
            let property = node.child_by_field_name("property")?;
            object
                .as_object()?
                .get(node_text(property, source))
                .cloned()
        }
        "subscript_expression" => {
            let object = static_value(
                node.child_by_field_name("object")?,
                source,
                constants,
                depth + 1,
            )?;
            let index = static_value(
                node.child_by_field_name("index")?,
                source,
                constants,
                depth + 1,
            )?;
            match object {
                Value::Object(object) => object.get(&value_to_string(index)?).cloned(),
                Value::Array(array) => index
                    .as_u64()
                    .and_then(|index| array.get(index as usize))
                    .cloned(),
                _ => None,
            }
        }
        "call_expression" => {
            let function_name = call_method_name(node, source)?;
            if !is_translation_call_name(&function_name) {
                return None;
            }
            let key = call_arguments(node)
                .first()
                .and_then(|argument| static_value(*argument, source, constants, depth + 1))
                .and_then(value_to_string)?;
            constants
                .get(&format!("{I18N_CONSTANT_PREFIX}{key}"))
                .cloned()
                .or_else(|| Some(Value::String(format!("{UNRESOLVED_I18N_PREFIX}{key}"))))
        }
        "parenthesized_expression" => node
            .named_child(0)
            .and_then(|child| static_value(child, source, constants, depth + 1)),
        "unary_expression" => {
            let argument = node.child_by_field_name("argument")?;
            let value = static_value(argument, source, constants, depth + 1)?;
            let text = node_text(node, source).trim_start();
            if text.starts_with('-') {
                Number::from_f64(-value.as_f64()?).map(Value::Number)
            } else if text.starts_with('+') {
                Some(value)
            } else {
                None
            }
        }
        "array" => {
            let mut values = Vec::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "spread_element" {
                    return None;
                }
                values.push(static_value(child, source, constants, depth + 1)?);
            }
            Some(Value::Array(values))
        }
        "object" => {
            let mut object = Map::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "pair" {
                    return None;
                }
                let key = property_key(child.child_by_field_name("key")?, source)?;
                let value = static_value(
                    child.child_by_field_name("value")?,
                    source,
                    constants,
                    depth + 1,
                )?;
                object.insert(key, value);
            }
            Some(Value::Object(object))
        }
        _ => None,
    }
}

fn static_control_value(
    node: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    program_index: &SettingsProgramIndex<'_>,
    depth: usize,
    call_stack: &mut BTreeSet<String>,
) -> Option<Value> {
    if depth > 20 {
        return None;
    }
    if let Some(value) = static_value(node, source, constants, depth) {
        return Some(value);
    }

    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            let value = program_index.values.get(name).copied()?;
            static_control_value(
                value,
                source,
                constants,
                program_index,
                depth + 1,
                call_stack,
            )
        }
        "call_expression" => {
            if call_receiver_identifier(node, source).as_deref() == Some("Object")
                && call_method_name(node, source).as_deref() == Some("keys")
            {
                let argument = call_arguments(node).first().copied()?;
                return static_object_keys(
                    argument,
                    source,
                    constants,
                    program_index,
                    depth + 1,
                    call_stack,
                );
            }

            let function = node.child_by_field_name("function")?;
            if function.kind() != "identifier" {
                return None;
            }
            let name = node_text(function, source).to_string();
            let callable = program_index.functions.get(&name).copied()?;
            if !call_stack.insert(name.clone()) {
                return None;
            }
            let arguments = call_arguments(node)
                .into_iter()
                .map(|argument| {
                    static_control_value(
                        argument,
                        source,
                        constants,
                        program_index,
                        depth + 1,
                        call_stack,
                    )
                })
                .collect::<Option<Vec<_>>>();
            let result = arguments.and_then(|arguments| {
                evaluate_pure_function(
                    callable,
                    &arguments,
                    source,
                    constants,
                    program_index,
                    depth + 1,
                    call_stack,
                )
            });
            call_stack.remove(&name);
            result
        }
        "binary_expression" => {
            let left = node.child_by_field_name("left")?;
            let right = node.child_by_field_name("right")?;
            let operator = source.get(left.end_byte()..right.start_byte())?.trim();
            match operator {
                "||" => {
                    let left_value = static_control_value(
                        left,
                        source,
                        constants,
                        program_index,
                        depth + 1,
                        call_stack,
                    );
                    if left_value.as_ref().is_some_and(js_truthy) {
                        left_value
                    } else {
                        static_control_value(
                            right,
                            source,
                            constants,
                            program_index,
                            depth + 1,
                            call_stack,
                        )
                    }
                }
                "??" => {
                    let left_value = static_control_value(
                        left,
                        source,
                        constants,
                        program_index,
                        depth + 1,
                        call_stack,
                    );
                    match left_value {
                        Some(Value::Null) | None => static_control_value(
                            right,
                            source,
                            constants,
                            program_index,
                            depth + 1,
                            call_stack,
                        ),
                        value => value,
                    }
                }
                _ => None,
            }
        }
        "parenthesized_expression" => node.named_child(0).and_then(|child| {
            static_control_value(
                child,
                source,
                constants,
                program_index,
                depth + 1,
                call_stack,
            )
        }),
        _ => None,
    }
}

fn static_object_keys(
    argument: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
    program_index: &SettingsProgramIndex<'_>,
    depth: usize,
    call_stack: &mut BTreeSet<String>,
) -> Option<Value> {
    let object_node = if argument.kind() == "identifier" {
        program_index
            .values
            .get(node_text(argument, source))
            .copied()
    } else {
        Some(argument)
    };
    if let Some(object_node) = object_node.filter(|node| node.kind() == "object") {
        let mut keys = Vec::new();
        let mut cursor = object_node.walk();
        for child in object_node.named_children(&mut cursor) {
            if child.kind() != "pair" {
                return None;
            }
            let key = property_key(child.child_by_field_name("key")?, source)?;
            keys.push(Value::String(key));
        }
        return Some(Value::Array(keys));
    }

    static_control_value(
        argument,
        source,
        constants,
        program_index,
        depth + 1,
        call_stack,
    )
    .and_then(|value| {
        value
            .as_object()
            .map(|object| Value::Array(object.keys().cloned().map(Value::String).collect()))
    })
}

#[allow(clippy::too_many_arguments)]
fn evaluate_pure_function(
    callable: Node<'_>,
    arguments: &[Value],
    source: &str,
    constants: &HashMap<String, Value>,
    program_index: &SettingsProgramIndex<'_>,
    depth: usize,
    call_stack: &mut BTreeSet<String>,
) -> Option<Value> {
    let parameters = callable_parameter_names(callable, source);
    if parameters.len() != arguments.len() {
        return None;
    }
    let mut scope = constants.clone();
    scope.extend(parameters.into_iter().zip(arguments.iter().cloned()));

    let body = callable.child_by_field_name("body")?;
    if body.kind() != "statement_block" {
        return static_control_value(body, source, &scope, program_index, depth + 1, call_stack);
    }

    let mut cursor = body.walk();
    for statement in body.named_children(&mut cursor) {
        match statement.kind() {
            "lexical_declaration" | "variable_declaration" => {
                let mut declaration_cursor = statement.walk();
                for declaration in statement.named_children(&mut declaration_cursor) {
                    if declaration.kind() != "variable_declarator" {
                        return None;
                    }
                    let name = declaration.child_by_field_name("name")?;
                    let value = declaration.child_by_field_name("value")?;
                    if name.kind() != "identifier" {
                        return None;
                    }
                    let value = static_control_value(
                        value,
                        source,
                        &scope,
                        program_index,
                        depth + 1,
                        call_stack,
                    )?;
                    scope.insert(node_text(name, source).to_string(), value);
                }
            }
            "return_statement" => {
                let value = statement.named_child(0)?;
                return static_control_value(
                    value,
                    source,
                    &scope,
                    program_index,
                    depth + 1,
                    call_stack,
                );
            }
            _ => return None,
        }
    }
    None
}

fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value
            .as_f64()
            .is_some_and(|value| value != 0.0 && !value.is_nan()),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn is_translation_call_name(name: &str) -> bool {
    name == "t"
        || name.eq_ignore_ascii_case("translate")
        || name.strip_prefix('t').is_some_and(|suffix| {
            !suffix.is_empty() && suffix.chars().all(|char| char.is_ascii_digit())
        })
}

fn first_call_value(
    calls: &[Node<'_>],
    method: &str,
    source: &str,
    constants: &HashMap<String, Value>,
) -> Option<Value> {
    calls.iter().find_map(|call| {
        if call_method_name(*call, source).as_deref() != Some(method) {
            return None;
        }
        call_arguments(*call)
            .first()
            .and_then(|argument| static_value(*argument, source, constants, 0))
    })
}

fn call_method_name(call: Node<'_>, source: &str) -> Option<String> {
    let function = call.child_by_field_name("function")?;
    match function.kind() {
        "member_expression" => function
            .child_by_field_name("property")
            .map(|property| node_text(property, source).to_string()),
        "identifier" => Some(node_text(function, source).to_string()),
        _ => None,
    }
}

fn call_arguments(call: Node<'_>) -> Vec<Node<'_>> {
    let Some(arguments) = call.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut cursor = arguments.walk();
    arguments.named_children(&mut cursor).collect()
}

fn is_setting_constructor(node: Node<'_>, source: &str, aliases: &ObsidianAliases) -> bool {
    node.child_by_field_name("constructor")
        .map(|constructor| {
            let text = node_text(constructor, source);
            let base = text.rsplit('.').next().unwrap_or(text);
            aliases.settings.contains(base)
        })
        .unwrap_or(false)
}

fn expression_scope(mut node: Node<'_>) -> Node<'_> {
    while let Some(parent) = node.parent() {
        let belongs_to_chain = match parent.kind() {
            "member_expression" | "subscript_expression" => {
                parent.child_by_field_name("object") == Some(node)
            }
            "call_expression" => parent.child_by_field_name("function") == Some(node),
            "parenthesized_expression" | "optional_chain" => true,
            _ => false,
        };
        if !belongs_to_chain {
            break;
        }
        node = parent;
    }
    node
}

fn pointer_from_expression(
    node: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
) -> Option<String> {
    let segments = member_segments(node, source, constants)?;
    let marker = segments.iter().rposition(|segment| {
        matches!(
            segment.to_ascii_lowercase().as_str(),
            "settings" | "config" | "configuration" | "options" | "data"
        )
    })?;
    let tail = &segments[marker + 1..];
    if tail.is_empty() {
        return None;
    }
    Some(pointer_from_segments(tail))
}

fn pointer_candidates_from_expression(
    node: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
) -> BTreeSet<String> {
    let mut candidates = BTreeSet::new();
    collect_nodes(node, &mut |candidate| {
        if !matches!(
            candidate.kind(),
            "member_expression" | "subscript_expression"
        ) {
            return;
        }
        if candidate.kind() == "member_expression"
            && candidate.parent().is_some_and(|parent| {
                parent.kind() == "call_expression"
                    && parent.child_by_field_name("function") == Some(candidate)
            })
            && candidate
                .child_by_field_name("object")
                .is_some_and(|object| pointer_from_expression(object, source, constants).is_some())
        {
            return;
        }
        if let Some(pointer) = pointer_from_expression(candidate, source, constants) {
            candidates.insert(pointer);
        }
    });
    let all = candidates.iter().cloned().collect::<Vec<_>>();
    candidates.retain(|candidate| {
        let nested_prefix = format!("{candidate}/");
        !all.iter().any(|other| other.starts_with(&nested_prefix))
    });
    candidates
}

fn member_segments(
    node: Node<'_>,
    source: &str,
    constants: &HashMap<String, Value>,
) -> Option<Vec<String>> {
    match node.kind() {
        "identifier" | "property_identifier" | "this" => {
            Some(vec![node_text(node, source).to_string()])
        }
        "member_expression" => {
            let mut segments =
                member_segments(node.child_by_field_name("object")?, source, constants)?;
            let property = node.child_by_field_name("property")?;
            segments.push(node_text(property, source).to_string());
            Some(segments)
        }
        "subscript_expression" => {
            let mut segments =
                member_segments(node.child_by_field_name("object")?, source, constants)?;
            let index = node.child_by_field_name("index")?;
            let value = static_value(index, source, constants, 0)?;
            segments.push(value_to_string(value)?);
            Some(segments)
        }
        "parenthesized_expression" => member_segments(node.named_child(0)?, source, constants),
        _ => None,
    }
}

fn pointer_for_key(key: &str) -> String {
    pointer_from_segments(&[key.to_string()])
}

fn pointer_from_segments(segments: &[String]) -> String {
    segments
        .iter()
        .map(|segment| format!("/{}", escape_pointer_segment(segment)))
        .collect()
}

fn escape_pointer_segment(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn pointer_last_segment(pointer: &str) -> Option<String> {
    pointer
        .rsplit('/')
        .next()
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
}

fn parse_js_string(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let quote = chars.next()?;
    if !matches!(quote, '\'' | '"' | '`') || !text.ends_with(quote) {
        return None;
    }
    let body = &text[quote.len_utf8()..text.len() - quote.len_utf8()];
    let mut output = String::new();
    let mut chars = body.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        let escaped = chars.next()?;
        match escaped {
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            'b' => output.push('\u{0008}'),
            'f' => output.push('\u{000C}'),
            'v' => output.push('\u{000B}'),
            '0' => output.push('\0'),
            '\n' => {}
            'u' => {
                let hex: String = chars.by_ref().take(4).collect();
                let value = u32::from_str_radix(&hex, 16).ok()?;
                output.push(char::from_u32(value)?);
            }
            'x' => {
                let hex: String = chars.by_ref().take(2).collect();
                let value = u8::from_str_radix(&hex, 16).ok()?;
                output.push(char::from(value));
            }
            other => output.push(other),
        }
    }
    Some(output)
}

fn humanize_key(key: &str) -> String {
    if key.is_empty() {
        return "配置".to_string();
    }
    let mut output = String::new();
    let mut previous_lower_or_digit = false;
    for character in key.chars() {
        if matches!(character, '_' | '-' | '.') {
            if !output.ends_with(' ') {
                output.push(' ');
            }
            previous_lower_or_digit = false;
            continue;
        }
        if character.is_ascii_uppercase() && previous_lower_or_digit && !output.ends_with(' ') {
            output.push(' ');
        }
        previous_lower_or_digit = character.is_ascii_lowercase() || character.is_ascii_digit();
        output.push(character);
    }
    output.trim().to_string()
}

fn value_to_string(value: Value) -> Option<String> {
    match value {
        Value::String(value) => Some(
            value
                .strip_prefix(UNRESOLVED_I18N_PREFIX)
                .map(humanize_translation_key)
                .unwrap_or(value),
        ),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn is_unresolved_i18n_value(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|value| value.starts_with(UNRESOLVED_I18N_PREFIX))
}

fn humanize_translation_key(key: &str) -> String {
    let mut segments = key.split('.').collect::<Vec<_>>();
    if matches!(
        segments.last().copied(),
        Some("name" | "label" | "title" | "desc" | "description")
    ) && segments.len() > 1
    {
        segments.pop();
    }
    humanize_key(segments.last().copied().unwrap_or(key))
}

fn value_to_string_vec(value: &Value) -> Vec<String> {
    match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(values) => values
            .iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

fn display_json_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or_default()
}

fn collect_nodes<'tree>(node: Node<'tree>, visit: &mut impl FnMut(Node<'tree>)) {
    visit(node);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_nodes(child, visit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PluginRuntimeSettingField;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn schema(source: &str, configuration: Value) -> PluginSettingsSchema {
        infer_settings_schema(Some(source), Some(&configuration), Vec::new())
    }

    fn registered(source: &str, tab: &str) -> String {
        format!(
            "{source}\nclass TestPlugin extends Plugin {{ onload() {{ this.addSettingTab(new {tab}(this.app, this)); }} }}"
        )
    }

    fn fields(schema: &PluginSettingsSchema) -> Vec<&PluginSettingField> {
        schema
            .groups
            .iter()
            .flat_map(|group| group.fields.iter())
            .collect()
    }

    #[test]
    fn extracts_declarative_settings_and_options() {
        let source = r#"
            class ExampleTab extends PluginSettingTab {
              getSettingDefinitions() {
                return [
                  { name: 'Enabled', desc: 'Turns it on.', control: { type: 'toggle', key: 'enabled' } },
                  { name: 'Mode', control: { type: 'dropdown', key: 'mode', options: { fast: 'Fast', slow: 'Slow' } } },
                  { name: 'Count', control: { type: 'slider', key: 'count', min: 1, max: 10, step: 1 } }
                ];
              }
            }
        "#;
        let result = schema(
            &registered(source, "ExampleTab"),
            json!({"enabled": true, "mode": "fast", "count": 3}),
        );
        let fields = fields(&result);
        assert_eq!(result.source, PluginSettingsSchemaSource::Declarative);
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].path.as_deref(), Some("/enabled"));
        assert_eq!(fields[1].options.len(), 2);
        assert_eq!(fields[2].min, Some(1.0));
        assert_eq!(fields[2].max, Some(10.0));
    }

    #[test]
    fn extracts_imperative_controls_and_nested_paths() {
        let source = r#"
          class ExampleTab extends obsidian.PluginSettingTab {
            display() {
              const { containerEl } = this;
              new obsidian.Setting(containerEl)
                .setName('Feature')
                .setDesc('Feature description')
                .addToggle(toggle => toggle
                  .setValue(this.plugin.settings.nested.enabled)
                  .onChange(value => { this.plugin.settings.nested.enabled = value; }));
              new obsidian.Setting(containerEl)
                .setName('Mode')
                .addDropdown(dropdown => dropdown
                  .addOption('a', 'Alpha')
                  .addOption('b', 'Beta')
                  .setValue(this.plugin.settings.mode)
                  .onChange(value => { this.plugin.settings.mode = value; }));
              new obsidian.Setting(containerEl)
                .setName('Size')
                .addSlider(slider => slider.setLimits(1, 9, 2).setValue(this.plugin.settings.size));
            }
          }
        "#;
        let result = schema(
            &registered(source, "ExampleTab"),
            json!({"nested": {"enabled": true}, "mode": "a", "size": 3}),
        );
        let fields = fields(&result);
        assert_eq!(result.source, PluginSettingsSchemaSource::Imperative);
        assert_eq!(fields[0].path.as_deref(), Some("/nested/enabled"));
        assert!(!fields[0].read_only);
        assert_eq!(fields[0].support, PluginSettingSupport::SafeWritable);
        assert_eq!(fields[1].options.len(), 2);
        assert!(!fields[1].read_only);
        assert_eq!(fields[1].support, PluginSettingSupport::SafeWritable);
        assert_eq!(fields[2].step, Some(2.0));
        assert!(fields[2].read_only);
        assert_eq!(fields[2].support, PluginSettingSupport::RiskTransform);
        assert_eq!(result.coverage.total, 3);
        assert_eq!(result.coverage.safe_writable, 2);
        assert_eq!(result.coverage.risk_transform, 1);
    }

    #[test]
    fn expands_static_dropdown_option_loops() {
        let source = r#"
          const MODES = [
            { value: 'fast', label: 'Fast' },
            { value: 'safe', label: 'Safe' }
          ];
          class T extends PluginSettingTab {
            display() {
              new Setting(this.containerEl).setName('Mode').addDropdown(dropdown => {
                for (const mode of MODES) {
                  dropdown.addOption(mode.value, mode.label);
                }
                dropdown.setValue(this.plugin.settings.mode)
                  .onChange(value => { this.plugin.settings.mode = value; });
              });
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({"mode": "safe"}));
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].options.len(), 2);
        assert_eq!(extracted[0].options[0].value, Value::String("fast".into()));
        assert_eq!(extracted[0].options[1].label, "Safe");
    }

    #[test]
    fn evaluates_pure_helpers_for_dropdown_values_and_labels() {
        let source = r#"
          const translations = {
            en: en_exports,
            'zh-CN': zh_exports,
            ja: ja_exports
          };
          function getAvailableLocales() {
            return Object.keys(translations);
          }
          function getLocaleDisplayName(locale) {
            const names = {
              en: 'English',
              'zh-CN': '简体中文',
              ja: '日本語'
            };
            return names[locale] || locale;
          }
          class T extends PluginSettingTab {
            display() {
              new Setting(this.containerEl).setName('Language').addDropdown(dropdown => {
                const locales = getAvailableLocales();
                for (const locale of locales) {
                  dropdown.addOption(locale, getLocaleDisplayName(locale));
                }
                dropdown.setValue(this.plugin.settings.locale)
                  .onChange(value => { this.plugin.settings.locale = value; });
              });
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({"locale": "zh-CN"}));
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].control, PluginSettingControl::Dropdown);
        assert_eq!(extracted[0].options.len(), 3);
        assert_eq!(extracted[0].options[0].value, json!("en"));
        assert_eq!(extracted[0].options[1].label, "简体中文");
        assert_eq!(extracted[0].options[2].label, "日本語");
    }

    #[test]
    fn preserves_partial_runtime_dropdown_semantics() {
        let source = r#"
          class T extends PluginSettingTab {
            display() {
              new Setting(this.containerEl).setName('Model').addDropdown(dropdown => {
                dropdown.addOption('', 'Auto');
                for (const model of ProviderRegistry.getModels()) {
                  dropdown.addOption(model.value, model.label);
                }
                dropdown.setValue(this.plugin.settings.model)
                  .onChange(value => { this.plugin.settings.model = value; });
              });
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({"model": "runtime-model"}));
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].control, PluginSettingControl::Dropdown);
        assert_eq!(extracted[0].options.len(), 1);
        assert_eq!(extracted[0].options[0].label, "Auto");
        assert!(extracted[0]
            .warnings
            .iter()
            .any(|warning| warning.contains("运行时来源")));
    }

    #[test]
    fn evaluates_pure_helpers_for_slider_limits() {
        let source = r#"
          function minimumTabs() { return 3; }
          const MIN_TABS = minimumTabs();
          const MAX_TABS = 10;
          const TAB_STEP = 1;
          class T extends PluginSettingTab {
            display() {
              new Setting(this.containerEl).setName('Maximum tabs').addSlider(slider => slider
                .setLimits(MIN_TABS, MAX_TABS, TAB_STEP)
                .setValue(this.plugin.settings.maxTabs)
                .onChange(value => { this.plugin.settings.maxTabs = value; }));
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({"maxTabs": 5}));
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].control, PluginSettingControl::Slider);
        assert_eq!(extracted[0].min, Some(3.0));
        assert_eq!(extracted[0].max, Some(10.0));
        assert_eq!(extracted[0].step, Some(1.0));
    }

    #[test]
    fn follows_registered_tab_helper_and_preserves_page() {
        let source = r#"
          class T extends PluginSettingTab {
            display() {
              const tabs = new Map();
              const general = this.containerEl.createDiv();
              tabs.set('general', general);
              this.renderGeneralTab(tabs.get('general'));
            }
            renderGeneralTab(container) {
              new Setting(container).setName('Visible').addToggle(toggle => toggle
                .setValue(this.plugin.settings.visible)
                .onChange(value => { this.plugin.settings.visible = value; }));
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({"visible": true}));
        let fields = fields(&result);

        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "Visible");
        assert_eq!(result.groups[0].page_path, vec!["general"]);
    }

    #[test]
    fn follows_top_level_and_called_local_helpers_only() {
        let source = r#"
          function renderSharedSection(options) {
            const { container, plugin } = options;
            new Setting(container).setName('Shared').addToggle(toggle => toggle
              .setValue(plugin.settings.shared)
              .onChange(value => { plugin.settings.shared = value; }));
          }
          class T extends PluginSettingTab {
            display() {
              const renderLocal = (target) => {
                new Setting(target).setName('Local').addText(text => text
                  .setValue(this.plugin.settings.local)
                  .onChange(value => { this.plugin.settings.local = value; }));
              };
              const neverCalled = (target) => {
                new Setting(target).setName('Internal').addText(text => text
                  .setValue(this.plugin.settings.internal)
                  .onChange(value => { this.plugin.settings.internal = value; }));
              };
              renderSharedSection({ container: this.containerEl, plugin: this.plugin });
              renderLocal(this.containerEl);
            }
          }
        "#;
        let result = schema(
            &registered(source, "T"),
            json!({"shared": true, "local": "yes", "internal": "no"}),
        );
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 2);
        assert!(extracted.iter().any(|field| field.name == "Shared"));
        assert!(extracted.iter().any(|field| field.name == "Local"));
        assert!(!extracted.iter().any(|field| field.name == "Internal"));
    }

    #[test]
    fn follows_registered_provider_renderers_into_separate_pages() {
        let source = r#"
          const alphaSettingsTabRenderer = {
            render(container, context) {
              new Setting(container).setName('Alpha option').addToggle(toggle => toggle
                .setValue(context.plugin.settings.alpha)
                .onChange(value => { context.plugin.settings.alpha = value; }));
            }
          };
          function createServices() {
            return { settingsTabRenderer: alphaSettingsTabRenderer };
          }
          class T extends PluginSettingTab {
            display() {
              const content = this.containerEl.createDiv();
              const renderer = Registry.getSettingsTabRenderer(providerId);
              renderer.render(content, { plugin: this.plugin });
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({"alpha": true}));
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].name, "Alpha option");
        assert_eq!(result.groups[0].page_path, vec!["alpha"]);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("settingsTabRenderer")));
    }

    #[test]
    fn resolves_provider_helpers_and_enumerates_existing_dynamic_keys() {
        let source = r#"
          function getProviderConfig(settings, providerId) {
            const candidate = settings.providerConfigs;
            const config = candidate[providerId];
            return { ...config };
          }
          function setProviderConfig(settings, providerId, config) {
            const next = { ...settings.providerConfigs };
            next[providerId] = { ...config };
            settings.providerConfigs = next;
          }
          function getCodexSettings(settings) {
            const config = getProviderConfig(settings, 'codex');
            const installationMethodsByHost = config.installationMethodsByHost ?? {};
            const hostnameKey = getHostnameKey();
            return {
              enabled: config.enabled ?? settings.codexEnabled ?? false,
              installationMethod: installationMethodsByHost[hostnameKey] ?? 'native-windows'
            };
          }
          function updateCodexSettings(settings, updates) {
            const current = getCodexSettings(settings);
            const installationMethodsByHost = { ...current.installationMethodsByHost };
            if ('installationMethod' in updates) {
              installationMethodsByHost[getHostnameKey()] = normalize(updates.installationMethod);
            }
            const next = { ...current, ...updates };
            setProviderConfig(settings, 'codex', {
              enabled: next.enabled,
              installationMethodsByHost
            });
          }
          function getConstantSettings(settings) {
            const config = getProviderConfig(settings, 'constant');
            return { enabled: config.enabled ?? false };
          }
          function updateConstantSettings(settings, updates) {
            setProviderConfig(settings, 'constant', { enabled: false });
          }
          const codexSettingsTabRenderer = {
            render(container, context) {
              const settingsBag = context.plugin.settings;
              const codexSettings = getCodexSettings(settingsBag);
              const constantSettings = getConstantSettings(settingsBag);
              let installationMethod = codexSettings.installationMethod;
              new Setting(container).setName('Enable Codex provider').addToggle(toggle => toggle
                .setValue(codexSettings.enabled)
                .onChange(value => updateCodexSettings(settingsBag, { enabled: value })));
              new Setting(container).setName('Installation method').addDropdown(dropdown => dropdown
                .addOption('native-windows', 'Native Windows')
                .addOption('wsl', 'WSL')
                .setValue(installationMethod)
                .onChange(value => {
                  installationMethod = value === 'wsl' ? 'wsl' : 'native-windows';
                  updateCodexSettings(settingsBag, { installationMethod });
                }));
              new Setting(container).setName('Unrelated same-name write').addToggle(toggle => toggle
                .setValue(constantSettings.enabled)
                .onChange(value => updateConstantSettings(settingsBag, { enabled: value })));
            }
          };
          function createCodexServices() {
            return { settingsTabRenderer: codexSettingsTabRenderer };
          }
          class T extends PluginSettingTab {
            display() {
              const content = this.containerEl.createDiv();
              Registry.getSettingsTabRenderer(providerId).render(content, { plugin: this.plugin });
            }
          }
        "#;
        let result = schema(
            &registered(source, "T"),
            json!({
                "providerConfigs": {
                    "codex": {
                        "enabled": true,
                        "installationMethodsByHost": {
                            "device:alpha": "wsl",
                            "device:a/b~c": "native-windows"
                        }
                    }
                },
                "codexEnabled": false
            }),
        );
        let extracted = fields(&result);
        let enabled = extracted
            .iter()
            .find(|field| field.name == "Enable Codex provider")
            .expect("provider enabled field");
        assert_eq!(
            enabled.path.as_deref(),
            Some("/providerConfigs/codex/enabled")
        );
        assert_eq!(
            enabled.read_paths,
            vec![
                "/providerConfigs/codex/enabled".to_string(),
                "/codexEnabled".to_string()
            ]
        );
        assert!(!enabled.read_only);

        let installation = extracted
            .iter()
            .find(|field| field.name == "Installation method")
            .expect("installation method field");
        assert!(installation.path.is_none());
        assert!(installation.read_only);
        assert_eq!(installation.path_options.len(), 2);
        assert!(installation.path_options.iter().any(|option| {
            option.path == "/providerConfigs/codex/installationMethodsByHost/device:a~1b~0c"
        }));

        let unrelated = extracted
            .iter()
            .find(|field| field.name == "Unrelated same-name write")
            .expect("unrelated write field");
        assert!(unrelated.path.is_none());
        assert!(unrelated.read_only);
    }

    #[test]
    fn dynamic_path_options_require_existing_object_keys() {
        assert!(enumerate_path_options(
            Some(&json!({"providerConfigs": {"codex": {}}})),
            "/providerConfigs/codex/byHost"
        )
        .is_empty());
        assert!(enumerate_path_options(
            Some(&json!({"providerConfigs": {"codex": {"byHost": []}}})),
            "/providerConfigs/codex/byHost"
        )
        .is_empty());
    }

    #[test]
    fn resolves_local_provider_object_and_marks_explicit_transform_risky() {
        let source = r#"
          function getProviderConfig(settings, providerId) {
            const candidate = settings.providerConfigs;
            return candidate[providerId] ?? {};
          }
          function setProviderConfig(settings, providerId, config) {
            const nextConfigs = { ...settings.providerConfigs };
            nextConfigs[providerId] = { ...config };
            settings.providerConfigs = nextConfigs;
          }
          function getClaudeSettings(settings) {
            const config = getProviderConfig(settings, 'claude');
            return {
              safeMode: normalize(config.safeMode ?? settings.claudeSafeMode),
              loadUserSettings: config.loadUserSettings ?? settings.loadUserClaudeSettings ?? false,
              customModels: config.customModels ?? ''
            };
          }
          function updateClaudeSettings(settings, updates) {
            const current = getClaudeSettings(settings);
            const next = {
              ...current,
              ...updates,
              safeMode: 'safeMode' in updates ? normalize(updates.safeMode) : current.safeMode
            };
            setProviderConfig(settings, 'claude', next);
          }
          class T extends PluginSettingTab {
            display() {
              const settingsBag = this.plugin.settings;
              const claudeSettings = getClaudeSettings(settingsBag);
              new Setting(this.containerEl).setName('Safe mode').addDropdown(dropdown => dropdown
                .setValue(claudeSettings.safeMode)
                .onChange(value => updateClaudeSettings(settingsBag, { safeMode: value })));
              new Setting(this.containerEl).setName('Load user settings').addToggle(toggle => toggle
                .setValue(claudeSettings.loadUserSettings)
                .onChange(value => updateClaudeSettings(settingsBag, { loadUserSettings: value })));
              new Setting(this.containerEl).setName('Custom models').addTextArea(text => {
                let pendingCustomModels = claudeSettings.customModels;
                const commit = async () => {
                  updateClaudeSettings(settingsBag, { customModels: pendingCustomModels });
                };
                text.setValue(claudeSettings.customModels).onChange(value => {
                  pendingCustomModels = value;
                });
                text.inputEl.addEventListener('blur', commit);
              });
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({}));
        let extracted = fields(&result);
        let safe_mode = extracted
            .iter()
            .find(|field| field.name == "Safe mode")
            .expect("safe mode");
        assert_eq!(
            safe_mode.path.as_deref(),
            Some("/providerConfigs/claude/safeMode")
        );
        assert!(safe_mode.read_only);

        let load_user = extracted
            .iter()
            .find(|field| field.name == "Load user settings")
            .expect("load user settings");
        assert_eq!(
            load_user.path.as_deref(),
            Some("/providerConfigs/claude/loadUserSettings")
        );
        assert!(!load_user.read_only);

        let custom_models = extracted
            .iter()
            .find(|field| field.name == "Custom models")
            .expect("custom models");
        assert_eq!(
            custom_models.path.as_deref(),
            Some("/providerConfigs/claude/customModels")
        );
        assert!(custom_models.read_only);
    }

    #[test]
    fn resolves_local_persist_helper_to_existing_dynamic_map_keys() {
        let source = r#"
          function getProviderConfig(settings, providerId) {
            return settings.providerConfigs[providerId] ?? {};
          }
          function setProviderConfig(settings, providerId, config) {
            const next = { ...settings.providerConfigs };
            next[providerId] = { ...config };
            settings.providerConfigs = next;
          }
          function getPiSettings(settings) {
            const config = getProviderConfig(settings, 'pi');
            return { cliPathsByHost: config.cliPathsByHost ?? {} };
          }
          function updatePiSettings(settings, updates) {
            const current = getPiSettings(settings);
            const next = { ...current, ...updates };
            setProviderConfig(settings, 'pi', { cliPathsByHost: next.cliPathsByHost });
          }
          class T extends PluginSettingTab {
            display() {
              const settingsBag = this.plugin.settings;
              const piSettings = getPiSettings(settingsBag);
              const hostnameKey = getHostnameKey();
              const cliPathsByHost = { ...piSettings.cliPathsByHost };
              const currentValue = piSettings.cliPathsByHost[hostnameKey] || '';
              const persistCliPath = async value => {
                const trimmed = value.trim();
                cliPathsByHost[hostnameKey] = trimmed;
                updatePiSettings(settingsBag, { cliPathsByHost: { ...cliPathsByHost } });
              };
              const cliSetting = new Setting(this.containerEl).setName('CLI path');
              cliSetting.addText(text => text
                .setValue(currentValue)
                .onChange(value => { void persistCliPath(value); }));
            }
          }
        "#;
        let result = schema(
            &registered(source, "T"),
            json!({"providerConfigs": {"pi": {"cliPathsByHost": {
                "device:existing": "C:/pi.cmd"
            }}}}),
        );
        let field = fields(&result)[0];

        assert!(field.path.is_none());
        assert!(field.read_only);
        assert_eq!(field.path_options.len(), 1);
        assert_eq!(field.support, PluginSettingSupport::DynamicExistingKey);
        assert_eq!(
            field.path_options[0].path,
            "/providerConfigs/pi/cliPathsByHost/device:existing"
        );
    }

    #[test]
    fn evaluates_simple_visibility_and_marks_unknown_conditions_read_only() {
        let source = r#"
          class T extends PluginSettingTab {
            display() {
              if (this.plugin.settings.showAdvanced) {
                new Setting(this.containerEl).setName('Visible').addToggle(toggle => toggle
                  .setValue(this.plugin.settings.visible)
                  .onChange(value => { this.plugin.settings.visible = value; }));
              }
              if (this.plugin.settings.hideInternal) {
                new Setting(this.containerEl).setName('Hidden').addToggle(toggle => toggle
                  .setValue(this.plugin.settings.hidden)
                  .onChange(value => { this.plugin.settings.hidden = value; }));
              }
              if (runtimeCondition()) {
                new Setting(this.containerEl).setName('Conditional').addText(text => text
                  .setValue(this.plugin.settings.conditional)
                  .onChange(value => { this.plugin.settings.conditional = value; }));
              }
            }
          }
        "#;
        let result = schema(
            &registered(source, "T"),
            json!({
                "showAdvanced": true,
                "hideInternal": false,
                "visible": true,
                "hidden": false,
                "conditional": "value"
            }),
        );
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 2);
        assert!(extracted.iter().any(|field| field.name == "Visible"));
        assert!(!extracted.iter().any(|field| field.name == "Hidden"));
        let conditional = extracted
            .iter()
            .find(|field| field.name == "Conditional")
            .expect("unknown conditional field");
        assert!(conditional.read_only);
        assert_eq!(conditional.support, PluginSettingSupport::RiskTransform);
        assert!(conditional
            .warnings
            .iter()
            .any(|warning| warning.contains("显示条件")));
    }

    #[test]
    fn resolves_static_exported_translation_dictionary() {
        let source = r#"
          const englishSettings = {
            display: 'Display',
            language: { name: 'Language', desc: 'Choose the interface language.' }
          };
          const en_exports = {};
          __export(en_exports, { settings: () => englishSettings });
          const translations = { en: en_exports };
          const DEFAULT_LOCALE = 'en';
          function t10(key) { return key; }
          class T extends PluginSettingTab {
            display() {
              new Setting(this.containerEl).setName(t10('settings.display')).setHeading();
              new Setting(this.containerEl)
                .setName(t10('settings.language.name'))
                .setDesc(t10('settings.language.desc'))
                .addText(text => text
                  .setValue(this.plugin.settings.locale)
                  .onChange(value => { this.plugin.settings.locale = value; }));
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({"locale": "en"}));
        let extracted = fields(&result);

        assert_eq!(result.groups[0].title.as_deref(), Some("Display"));
        assert_eq!(extracted[0].name, "Language");
        assert_eq!(
            extracted[0].description.as_deref(),
            Some("Choose the interface language.")
        );
        assert!(!extracted[0]
            .warnings
            .iter()
            .any(|warning| warning.contains("翻译标签")));
    }

    #[test]
    fn extracts_pointer_from_bundled_nullish_expression() {
        let source = r#"
          class T extends PluginSettingTab {
            display() {
              let current;
              new Setting(this.containerEl).setName('Count').addSlider(slider => slider
                .setValue((current = this.plugin.settings.count) != null ? current : 3)
                .onChange(value => { this.plugin.settings.count = value; }));
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({"count": 5}));
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].path.as_deref(), Some("/count"));
        assert!(!extracted[0].read_only);
    }

    #[test]
    fn preserves_password_buttons_and_disabled_controls() {
        let source = r#"
          class T extends PluginSettingTab {
            display() {
              new Setting(this.containerEl).setName('Secret').addText(text => {
                text.inputEl.type = 'password';
                text.setValue(this.plugin.settings.secret)
                  .onChange(value => { this.plugin.settings.secret = value; });
              });
              new Setting(this.containerEl).setName('Locked').addToggle(toggle => toggle
                .setDisabled(true)
                .setValue(this.plugin.settings.locked)
                .onChange(value => { this.plugin.settings.locked = value; }));
              new Setting(this.containerEl).setName('Run action').addButton(button => button
                .setButtonText('Run')
                .onClick(() => runAction()));
            }
          }
        "#;
        let result = schema(
            &registered(source, "T"),
            json!({"secret": "token", "locked": true}),
        );
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 3);
        assert_eq!(extracted[0].control, PluginSettingControl::Password);
        assert!(!extracted[0].read_only);
        assert!(extracted[1].read_only);
        assert_eq!(extracted[1].support, PluginSettingSupport::RiskTransform);
        assert_eq!(extracted[2].control, PluginSettingControl::Unsupported);
        assert!(extracted[2].read_only);
        assert_eq!(extracted[2].support, PluginSettingSupport::ActionOnly);
    }

    #[test]
    fn classifies_unresolved_runtime_and_custom_controls() {
        let source = r#"
          class T extends PluginSettingTab {
            display() {
              const runtimeValue = getRuntimeValue();
              new Setting(this.containerEl).setName('Runtime path').addText(text => text
                .setValue(runtimeValue)
                .onChange(value => persistRuntimeValue(value)));
              new Setting(this.containerEl).setName('Custom panel').addComponent(component => {
                renderCustomPanel(component);
              });
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({}));
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 2);
        assert_eq!(
            extracted[0].support,
            PluginSettingSupport::UnresolvedRuntime
        );
        assert_eq!(
            extracted[1].support,
            PluginSettingSupport::UnsupportedCustom
        );
        assert_eq!(result.coverage.unresolved_runtime, 1);
        assert_eq!(result.coverage.unsupported_custom, 1);
    }

    #[test]
    fn value_control_takes_precedence_over_auxiliary_buttons() {
        let source = r#"
          class T extends PluginSettingTab {
            display() {
              new Setting(this.containerEl).setName('Path')
                .addButton(button => button.setButtonText('Browse').onClick(() => browse()))
                .addText(text => text
                  .setValue(this.plugin.settings.path)
                  .onChange(value => { this.plugin.settings.path = value; }));
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({"path": "C:/tools"}));
        let field = fields(&result)[0];

        assert_eq!(field.control, PluginSettingControl::Text);
        assert_eq!(field.path.as_deref(), Some("/path"));
        assert_eq!(field.support, PluginSettingSupport::SafeWritable);
    }

    #[test]
    fn runtime_presentation_enriches_controls_without_granting_writes() {
        let source = r#"
          class T extends PluginSettingTab {
            display() {
              new Setting(this.containerEl).setName('Mode').addDropdown(dropdown => dropdown
                .setValue(this.plugin.settings.mode)
                .onChange(value => { this.plugin.settings.mode = normalize(value); }));
            }
          }
        "#;
        let mut result = schema(&registered(source, "T"), json!({"mode": "safe"}));
        let original = fields(&result)[0].clone();
        let snapshot = PluginRuntimeSettingsSnapshot {
            protocol_version: 1,
            plugin_id: "example".to_string(),
            plugin_version: Some("1.0.0".to_string()),
            fields: vec![PluginRuntimeSettingField {
                page_path: Vec::new(),
                group_title: None,
                order: 0,
                name: "Mode".to_string(),
                description: Some("Runtime description".to_string()),
                control: PluginSettingControl::Dropdown,
                options: vec![
                    PluginSettingOption {
                        value: json!("safe"),
                        label: "Safe".to_string(),
                    },
                    PluginSettingOption {
                        value: json!("fast"),
                        label: "Fast".to_string(),
                    },
                ],
                placeholder: None,
                min: None,
                max: None,
                step: None,
                disabled: false,
                visible: true,
                action: false,
                confidence: PluginSettingConfidence::Exact,
            }],
            warnings: Vec::new(),
        };

        assert_eq!(
            merge_runtime_settings_presentation(&mut result, &snapshot),
            1
        );
        let merged = fields(&result)[0];
        assert_eq!(merged.description.as_deref(), Some("Runtime description"));
        assert_eq!(merged.options.len(), 2);
        assert_eq!(merged.path, original.path);
        assert_eq!(merged.read_only, original.read_only);
        assert_eq!(merged.support, original.support);
        assert_eq!(merged.support, PluginSettingSupport::RiskTransform);
    }

    #[test]
    fn runtime_presentation_skips_ambiguous_matches() {
        let source = r#"
          class T extends PluginSettingTab {
            display() {
              new Setting(this.containerEl).setName('Mode').addText(text => text
                .setValue(this.plugin.settings.first)
                .onChange(value => { this.plugin.settings.first = value; }));
              new Setting(this.containerEl).setName('Mode').addText(text => text
                .setValue(this.plugin.settings.second)
                .onChange(value => { this.plugin.settings.second = value; }));
            }
          }
        "#;
        let mut result = schema(
            &registered(source, "T"),
            json!({"first": "a", "second": "b"}),
        );
        let snapshot = PluginRuntimeSettingsSnapshot {
            protocol_version: 1,
            plugin_id: "example".to_string(),
            plugin_version: None,
            fields: vec![PluginRuntimeSettingField {
                page_path: Vec::new(),
                group_title: None,
                order: 99,
                name: "Mode".to_string(),
                description: Some("Ambiguous".to_string()),
                control: PluginSettingControl::Text,
                options: Vec::new(),
                placeholder: None,
                min: None,
                max: None,
                step: None,
                disabled: false,
                visible: true,
                action: false,
                confidence: PluginSettingConfidence::Exact,
            }],
            warnings: Vec::new(),
        };

        assert_eq!(
            merge_runtime_settings_presentation(&mut result, &snapshot),
            0
        );
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("匹配到多个静态字段")));
        let all_fields = fields(&result);
        let runtime = all_fields
            .iter()
            .find(|field| field.description.as_deref() == Some("Ambiguous"))
            .expect("ambiguous runtime row");
        assert!(runtime.read_only);
        assert!(runtime.path.is_none());
        assert_eq!(runtime.support, PluginSettingSupport::UnresolvedRuntime);
    }

    #[test]
    fn ignores_settings_outside_registered_tab_call_graph() {
        let source = r#"
          class UnrelatedModal {
            display() {
              new Setting(this.containerEl).setName('Internal').addText(text => text
                .setValue(this.plugin.settings.internal)
                .onChange(value => { this.plugin.settings.internal = value; }));
            }
          }
          class T extends PluginSettingTab {
            display() {
              new Setting(this.containerEl).setName('Visible').addToggle(toggle => toggle
                .setValue(this.plugin.settings.visible)
                .onChange(value => { this.plugin.settings.visible = value; }));
            }
          }
        "#;
        let result = schema(
            &registered(source, "T"),
            json!({"visible": true, "internal": "hidden"}),
        );
        let fields = fields(&result);

        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path.as_deref(), Some("/visible"));
    }

    #[test]
    fn unregistered_setting_tab_is_not_an_extraction_root() {
        let source = r#"
          class T extends PluginSettingTab {
            display() {
              new Setting(this.containerEl).setName('Not registered').addToggle(toggle => toggle
                .setValue(this.plugin.settings.visible)
                .onChange(value => { this.plugin.settings.visible = value; }));
            }
          }
        "#;
        let result = schema(source, json!({"visible": true}));

        assert!(fields(&result).is_empty());
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("addSettingTab")));
    }

    #[test]
    fn parses_minified_single_line_settings() {
        let source = "class T extends o.PluginSettingTab{display(){let{containerEl:e}=this;new o.Setting(e).setName('Flag').addToggle(t=>t.setValue(this.plugin.settings.flag).onChange(e=>this.plugin.settings.flag=e))}}";
        let result = schema(&registered(source, "T"), json!({"flag": false}));
        let fields = fields(&result);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "Flag");
        assert_eq!(fields[0].path.as_deref(), Some("/flag"));
    }

    #[test]
    fn resolves_destructured_obsidian_api_aliases() {
        let source = r#"
          const { PluginSettingTab: PST, Setting: S } = require('obsidian');
          class T extends PST {
            display() {
              new S(this.containerEl).setName('Aliased').addToggle(toggle => toggle
                .setValue(this.plugin.settings.aliased)
                .onChange(value => { this.plugin.settings.aliased = value; }));
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({"aliased": true}));
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].name, "Aliased");
        assert!(!extracted[0].read_only);
    }

    #[test]
    fn resolves_named_obsidian_import_aliases() {
        let source = r#"
          import { PluginSettingTab as PST, Setting as S } from 'obsidian';
          class T extends PST {
            display() {
              new S(this.containerEl).setName('Imported').addText(text => text
                .setValue(this.plugin.settings.imported)
                .onChange(value => { this.plugin.settings.imported = value; }));
            }
          }
        "#;
        let result = schema(&registered(source, "T"), json!({"imported": "yes"}));
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].name, "Imported");
        assert!(!extracted[0].read_only);
    }

    #[test]
    fn separates_setting_chains_inside_minified_comma_expression() {
        let source = "class T extends o.PluginSettingTab{display(){let{containerEl:e}=this;new o.Setting(e).setName('First').addToggle(t=>t.setValue(this.plugin.settings.first).onChange(v=>this.plugin.settings.first=v)),new o.Setting(e).setName('Second').addText(t=>t.setValue(this.plugin.settings.second).onChange(v=>this.plugin.settings.second=v))}}";
        let result = schema(
            &registered(source, "T"),
            json!({"first": false, "second": "value"}),
        );
        let extracted = fields(&result);

        assert_eq!(extracted.len(), 2);
        assert_eq!(extracted[0].name, "First");
        assert_eq!(extracted[1].name, "Second");
        assert!(extracted.iter().all(|field| !field.read_only));
    }

    #[test]
    fn keeps_ambiguous_imperative_setting_read_only() {
        let source = r#"
          class T extends PluginSettingTab { display() {
            const { containerEl } = this;
            new Setting(containerEl).setName('Combined').addText(text => text
              .setValue(this.plugin.settings.first)
              .onChange(value => { this.plugin.settings.second = value; }));
          }}
        "#;
        let result = schema(
            &registered(source, "T"),
            json!({"first": "a", "second": "b"}),
        );
        let extracted = fields(&result)
            .into_iter()
            .find(|field| field.source == PluginSettingSource::Imperative)
            .expect("imperative field");
        assert!(extracted.read_only);
        assert_eq!(extracted.path.as_deref(), Some("/first"));
        assert_eq!(extracted.support, PluginSettingSupport::RiskTransform);
    }

    #[test]
    fn transformed_on_change_remains_read_only() {
        let source = r#"
          class T extends PluginSettingTab { display() {
            new Setting(this.containerEl).setName('Trimmed').addText(text => text
              .setValue(this.plugin.settings.name)
              .onChange(value => { this.plugin.settings.name = value.trim(); }));
          }}
        "#;
        let result = schema(&registered(source, "T"), json!({"name": "demo"}));
        let extracted = fields(&result)[0];

        assert_eq!(extracted.path.as_deref(), Some("/name"));
        assert!(extracted.read_only);
        assert!(extracted
            .warnings
            .iter()
            .any(|warning| warning.contains("值转换")));
    }

    #[test]
    fn method_calls_use_the_receiver_as_the_read_path() {
        let source = r#"
          class T extends PluginSettingTab { display() {
            new Setting(this.containerEl).setName('Tags').addTextArea(text => text
              .setValue(this.plugin.settings.excludedTags.join('\n'))
              .onChange(value => {
                this.plugin.settings.excludedTags = value.split(/\r?\n/).filter(Boolean);
              }));
          }}
        "#;
        let result = schema(
            &registered(source, "T"),
            json!({"excludedTags": ["private"]}),
        );
        let extracted = fields(&result)[0];

        assert_eq!(extracted.path.as_deref(), Some("/excludedTags"));
        assert!(extracted.read_only);
    }

    #[test]
    fn omits_data_json_fields_without_setting_evidence() {
        let source = r#"
          class T extends PluginSettingTab { getSettingDefinitions() {
            return [{ name: 'Known', control: { type: 'toggle', key: 'known' } }];
          }}
        "#;
        let result = schema(
            &registered(source, "T"),
            json!({"known": true, "extra": {"count": 2}}),
        );
        let fields = fields(&result);
        assert_eq!(
            fields
                .iter()
                .filter(|field| field.path.as_deref() == Some("/known"))
                .count(),
            1
        );
        assert!(!fields
            .iter()
            .any(|field| field.path.as_deref() == Some("/extra/count")));
        assert_eq!(result.completeness, PluginSettingsCompleteness::Complete);
    }

    #[test]
    fn keeps_proven_dynamic_rows_read_only_without_json_fallback() {
        let source = r#"
          class T extends PluginSettingTab { display() {
            const { containerEl } = this;
            new Setting(containerEl).setName(t('Dynamic heading')).setHeading();
            new Setting(containerEl).setName(t('Dynamic field')).addText(text => text.setValue(runtimeValue));
          }}
        "#;
        let result = schema(
            &registered(source, "T"),
            json!({"personalAccessToken": "secret-value"}),
        );
        let fields = fields(&result);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "Dynamic field");
        assert!(fields[0].read_only);
        assert!(fields[0].path.is_none());
        assert!(!fields
            .iter()
            .any(|field| field.name.contains("personalAccessToken")));
    }

    #[test]
    fn malformed_source_does_not_expose_data_json() {
        let result = schema("class Broken {", json!({"enabled": true}));
        let fields = fields(&result);
        assert_eq!(result.source, PluginSettingsSchemaSource::DataJson);
        assert_eq!(result.completeness, PluginSettingsCompleteness::Fallback);
        assert!(fields.is_empty());
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("无法完整解析")));
    }

    #[test]
    fn declarative_definitions_take_precedence_over_display() {
        let source = r#"
          class T extends PluginSettingTab {
            getSettingDefinitions() {
              return [{ name: 'Current', control: { type: 'toggle', key: 'current' } }];
            }
            display() {
              new Setting(this.containerEl).setName('Legacy').addToggle(toggle =>
                toggle.setValue(this.plugin.settings.legacy)
                  .onChange(value => { this.plugin.settings.legacy = value; }));
            }
          }
        "#;
        let result = schema(
            &registered(source, "T"),
            json!({"current": true, "legacy": false}),
        );
        let fields = fields(&result);

        assert_eq!(result.source, PluginSettingsSchemaSource::Declarative);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path.as_deref(), Some("/current"));
    }

    #[test]
    fn oversized_source_is_skipped_without_reading_contents() {
        let path = std::env::temp_dir().join(format!(
            "obsidian-plugin-sync-oversized-{}.js",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let file = fs::File::create(&path).expect("create sparse fixture");
        file.set_len(MAX_MAIN_JS_BYTES + 1)
            .expect("grow sparse fixture");
        drop(file);

        let mut warnings = Vec::new();
        let source = read_bounded_source(&path, &mut warnings).expect("bounded read");
        assert!(source.is_none());
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("已跳过设置分析")));

        fs::remove_file(path).expect("cleanup fixture");
    }

    #[test]
    fn parses_javascript_string_escapes() {
        assert_eq!(
            parse_js_string(r#"'hello\nworld'"#).as_deref(),
            Some("hello\nworld")
        );
        assert_eq!(
            parse_js_string(r#"'Fran\xE7ais'"#).as_deref(),
            Some("Français")
        );
        assert_eq!(parse_js_string(r#"`plain`"#).as_deref(), Some("plain"));
    }

    #[test]
    #[ignore = "set OPS_PLUGIN_MAIN_JS to run the read-only local plugin audit"]
    fn audits_real_plugin_from_environment() {
        let main_path = std::env::var("OPS_PLUGIN_MAIN_JS").expect("OPS_PLUGIN_MAIN_JS");
        let source = fs::read_to_string(main_path).expect("read plugin main.js");
        let configuration = std::env::var("OPS_PLUGIN_DATA_JSON")
            .ok()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|content| serde_json::from_str::<Value>(&content).ok())
            .unwrap_or_else(|| json!({}));
        let schema = infer_settings_schema(Some(&source), Some(&configuration), Vec::new());
        let extracted = fields(&schema);
        let editable = extracted.iter().filter(|field| !field.read_only).count();
        let readonly = extracted.len() - editable;

        eprintln!(
            "source={:?} completeness={:?} coverage={:?} fields={} editable={} readonly={} groups={} warnings={:?}",
            schema.source,
            schema.completeness,
            schema.coverage,
            extracted.len(),
            editable,
            readonly,
            schema.groups.len(),
            schema.warnings
        );
        for group in &schema.groups {
            eprintln!(
                "page={:?} title={:?} fields={}",
                group.page_path,
                group.title,
                group.fields.len()
            );
            for field in &group.fields {
                eprintln!(
                    "  name={:?} control={:?} support={:?} path={:?} read_paths={:?} options={:?} limits={:?}/{:?}/{:?} path_options={} read_only={}",
                    field.name,
                    field.control,
                    field.support,
                    field.path,
                    field.read_paths,
                    field.options,
                    field.min,
                    field.max,
                    field.step,
                    field.path_options.len(),
                    field.read_only
                );
            }
        }
    }
}
