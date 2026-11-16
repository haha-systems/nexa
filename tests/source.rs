use nexa::{
    source::{self, Host, SourceSpan},
    value::{Value, ValueType},
};

const TRIM_SOURCE: &str = include_str!("../examples/hello/memories/trim/src/main.nexa");
const GREET_SOURCE: &str = include_str!("../examples/hello/memories/greet/src/main.nexa");
const WRITE_SOURCE: &str = include_str!("../examples/hello/memories/write/src/main.nexa");

#[test]
fn trim_and_greet_fixtures_execute_from_source() {
    let trim = source::check("trim.nexa", TRIM_SOURCE).unwrap();
    let greeting = source::check("greet.nexa", GREET_SOURCE).unwrap();

    let name = trim
        .evaluate("trim", &[Value::String(" \tAda\n ".to_owned())])
        .unwrap();
    assert_eq!(name, Value::String("Ada".to_owned()));
    assert_eq!(
        greeting.evaluate("greet", &[name]).unwrap(),
        Value::String("Hello, Ada!".to_owned())
    );
}

#[test]
fn changed_source_and_helper_calls_change_the_result() {
    let text = r#"nexa 0.3
module example.changed

fn surround(name is String) -> String {
  return "[" + name + "]"
}

export fn greet(name is String) -> String {
  return surround(std.text.trim_ascii(name))
}
"#;
    let checked = source::check("changed.nexa", text).unwrap();
    assert_eq!(
        checked
            .evaluate("greet", &[Value::String("  Ada  ".to_owned())])
            .unwrap(),
        Value::String("[Ada]".to_owned())
    );
    assert_eq!(checked.module_name(), "example.changed");
    assert_eq!(checked.exports().len(), 1);
}

#[test]
fn calls_support_named_or_positional_arguments_without_mixing() {
    let text = r#"nexa 0.3
module example.named

fn combine(left is String, right is String) -> String {
  return left + right
}

export fn greet(name is String) -> String {
  return combine(right: "!", left: name)
}
"#;
    let checked = source::check("named.nexa", text).unwrap();
    assert_eq!(
        checked
            .evaluate("greet", &[Value::String("Ada".to_owned())])
            .unwrap(),
        Value::String("Ada!".to_owned())
    );

    let mixed = r#"nexa 0.3
module example.mixed
fn combine(left is String, right is String) -> String { return left + right }
export fn greet(name is String) -> String { return combine(name, right: "!") }
"#;
    assert_eq!(
        source::check("mixed.nexa", mixed)
            .unwrap_err()
            .diagnostic
            .code,
        "E_ARGUMENT_NAMING"
    );

    let named_std = r#"nexa 0.3
module example.named_std
export fn clean(name is String) -> String { return std.text.trim_ascii(text: name) }
"#;
    assert_eq!(
        source::check("named_std.nexa", named_std)
            .unwrap()
            .evaluate("clean", &[Value::String(" Ada ".to_owned())])
            .unwrap(),
        Value::String("Ada".to_owned())
    );
}

#[test]
fn trim_is_ascii_only_and_preserves_unicode_whitespace() {
    let trim = source::check("trim.nexa", TRIM_SOURCE).unwrap();
    assert_eq!(
        trim.evaluate(
            "trim",
            &[Value::String("\t\u{000b}Ada\u{000c}\r\n".to_owned())]
        )
        .unwrap(),
        Value::String("Ada".to_owned())
    );
    assert_eq!(
        trim.evaluate("trim", &[Value::String("\u{00a0}Ada\u{00a0}".to_owned())])
            .unwrap(),
        Value::String("\u{00a0}Ada\u{00a0}".to_owned())
    );
}

#[test]
fn wrong_types_unknown_names_and_unknown_exports_fail_before_evaluation() {
    let bad_return = r#"nexa 0.3
module example.bad_return
export fn greet(name is String) -> Unit { return name }
"#;
    assert_eq!(
        source::check("bad_return.nexa", bad_return)
            .unwrap_err()
            .diagnostic
            .code,
        "E_RETURN_TYPE"
    );

    let bad_argument = r#"nexa 0.3
module example.bad_argument
export fn greet(name is String) -> String { return std.text.trim_ascii(unit) }
"#;
    assert_eq!(
        source::check("bad_argument.nexa", bad_argument)
            .unwrap_err()
            .diagnostic
            .code,
        "E_ARGUMENT_TYPE"
    );

    let unknown = r#"nexa 0.3
module example.unknown
export fn greet(name is String) -> String { return missing(name) }
"#;
    assert_eq!(
        source::check("unknown.nexa", unknown)
            .unwrap_err()
            .diagnostic
            .code,
        "E_UNKNOWN_NAME"
    );

    let good = source::check("greet.nexa", GREET_SOURCE).unwrap();
    assert_eq!(
        good.contract("other").unwrap_err().diagnostic.code,
        "E_UNKNOWN_EXPORT"
    );
    assert_eq!(
        good.evaluate("greet", &[Value::Unit(())])
            .unwrap_err()
            .diagnostic
            .code,
        "E_ARGUMENT_TYPE"
    );
}

#[test]
fn pure_functions_reject_actions_and_action_effects_are_derived() {
    let impure_function = r#"nexa 0.3
module example.impure
export fn greet(name is String) -> Unit { return std.console.write_line(name) }
"#;
    let error = source::check("impure.nexa", impure_function).unwrap_err();
    assert_eq!(error.diagnostic.code, "E_EFFECT_IN_FN");

    let missing_effect = r#"nexa 0.3
module example.missing_effect
export action write(message is String) -> Unit effects {} {
  return std.console.write_line(message)
}
"#;
    assert_eq!(
        source::check("missing_effect.nexa", missing_effect)
            .unwrap_err()
            .diagnostic
            .code,
        "E_EFFECT_SET"
    );

    let unknown_effect = r#"nexa 0.3
module example.unknown_effect
export action request() -> Unit effects {network.http} { return unit }
"#;
    let error = source::check("unknown_effect.nexa", unknown_effect).unwrap_err();
    assert_eq!(error.diagnostic.code, "E_UNKNOWN_EFFECT");
    assert_eq!(
        error.diagnostic.span.unwrap().start,
        unknown_effect.find("network.http").unwrap() as u64
    );

    let checked = source::check("write.nexa", WRITE_SOURCE).unwrap();
    let contract = checked.contract("write").unwrap();
    assert_eq!(contract.kind, nexa::contracts::CallableKind::Action);
    assert_eq!(contract.result_type, ValueType::Unit);
    assert_eq!(contract.effects, vec!["io.stdout"]);
    assert_eq!(contract.required_bindings, vec!["std.console.write_line"]);
    assert_eq!(
        checked
            .evaluate("write", &[Value::String("hi".to_owned())])
            .unwrap_err()
            .diagnostic
            .code,
        "E_CALL_KIND"
    );
}

#[test]
fn action_dispatch_uses_host_and_passes_the_source_span() {
    let checked = source::check("write.nexa", WRITE_SOURCE).unwrap();
    let mut host = RecordingHost::default();
    let result = checked
        .evaluate_with("write", &[Value::String("hello".to_owned())], &mut host)
        .unwrap();

    assert_eq!(result, Value::Unit(()));
    assert_eq!(host.calls.len(), 1);
    assert_eq!(host.calls[0].0, "std.console.write_line");
    assert_eq!(host.calls[0].1, vec![Value::String("hello".to_owned())]);
    let call = WRITE_SOURCE.find("std.console.write_line").unwrap();
    assert_eq!(host.calls[0].2.start, call as u64);
    assert_eq!(
        host.calls[0].2.end,
        (WRITE_SOURCE
            .find("std.console.write_line(message)")
            .unwrap()
            + "std.console.write_line(message)".len()) as u64
    );
}

#[test]
fn named_action_arguments_are_evaluated_left_to_right_before_parameter_reordering() {
    let text = r#"nexa 0.3
module example.argument_order

action relay(left is Unit, right is Unit) -> Unit effects {io.stdout} {
  return left
}

export action send() -> Unit effects {io.stdout} {
  return relay(right: std.console.write_line("first"), left: std.console.write_line("second"))
}
"#;
    let checked = source::check("argument_order.nexa", text).unwrap();
    let mut host = RecordingHost::default();
    assert_eq!(
        checked.evaluate_with("send", &[], &mut host).unwrap(),
        Value::Unit(())
    );
    assert_eq!(host.calls.len(), 2);
    assert_eq!(host.calls[0].1, vec![Value::String("first".to_owned())]);
    assert_eq!(host.calls[1].1, vec![Value::String("second".to_owned())]);
}

#[test]
fn recursive_helpers_are_rejected_and_unsupported_syntax_is_diagnosed() {
    let recursive = r#"nexa 0.3
module example.recursive
fn first(name is String) -> String { return second(name) }
fn second(name is String) -> String { return first(name) }
export fn greet(name is String) -> String { return first(name) }
"#;
    assert_eq!(
        source::check("recursive.nexa", recursive)
            .unwrap_err()
            .diagnostic
            .code,
        "E_RECURSION"
    );

    let unsupported = r#"nexa 0.3
module example.unsupported
export fn greet(name is String) -> String { let local = name; return local }
"#;
    assert_eq!(
        source::check("unsupported.nexa", unsupported)
            .unwrap_err()
            .diagnostic
            .code,
        "E_UNSUPPORTED"
    );
}

#[test]
fn newlines_are_significant_and_deep_nesting_returns_a_diagnostic() {
    let split_operator = r#"nexa 0.3
module example.split
export fn greet(name is String) -> String {
  return "Hello"
    + name
}
"#;
    assert_eq!(
        source::check("split.nexa", split_operator)
            .unwrap_err()
            .diagnostic
            .code,
        "E_UNSUPPORTED"
    );

    let split_statement = r#"nexa 0.3
module example.split_statement
export fn greet(name is String) -> String {
  return "Hello"
  return name
}
"#;
    assert_eq!(
        source::check("split_statement.nexa", split_statement)
            .unwrap_err()
            .diagnostic
            .code,
        "E_UNSUPPORTED"
    );

    let comment_boundary = r#"nexa 0.3
module example.comment_boundary
export fn greet(name is String) -> String {
  return "Hello" /* the newline remains significant
  */ + name
}
"#;
    assert_eq!(
        source::check("comment_boundary.nexa", comment_boundary)
            .unwrap_err()
            .diagnostic
            .code,
        "E_UNSUPPORTED"
    );

    let continued_operator = r#"nexa 0.3
module example.continued
export fn greet(name is String) -> String {
  return "Hello, " +
    name
}
"#;
    assert_eq!(
        source::check("continued.nexa", continued_operator)
            .unwrap()
            .evaluate("greet", &[Value::String("Ada".to_owned())])
            .unwrap(),
        Value::String("Hello, Ada".to_owned())
    );

    let split_call = r#"nexa 0.3
module example.split_call
fn trim(name is String) -> String { return std.text.trim_ascii(name) }
export fn greet(name is String) -> String { return trim
  (name) }
"#;
    assert_eq!(
        source::check("split_call.nexa", split_call)
            .unwrap_err()
            .diagnostic
            .code,
        "E_UNSUPPORTED"
    );

    let nested = format!(
        "nexa 0.3\nmodule example.deep\nexport fn value() -> String {{ return {}\"x\"{} }}\n",
        "(".repeat(300),
        ")".repeat(300)
    );
    assert_eq!(
        source::check("deep.nexa", &nested)
            .unwrap_err()
            .diagnostic
            .code,
        "E_NESTING_LIMIT"
    );
}

#[test]
fn semicolons_are_statement_and_declaration_separators() {
    let text = "nexa 0.3; module example.semicolons; export fn greet(name is String) -> String { return name; }";
    assert_eq!(
        source::check("semicolons.nexa", text)
            .unwrap()
            .evaluate("greet", &[Value::String("Ada".to_owned())])
            .unwrap(),
        Value::String("Ada".to_owned())
    );
}

#[test]
fn string_escapes_and_triple_quoted_literals_follow_scalar_rules() {
    let text = r#"nexa 0.3
module example.strings
export fn value() -> String { return "A\u{1F600}\u{00000041}" }
"#;
    assert_eq!(
        source::check("strings.nexa", text)
            .unwrap()
            .evaluate("value", &[])
            .unwrap(),
        Value::String("A😀A".to_owned())
    );

    let triple = "nexa 0.3\nmodule example.triple\nexport fn value() -> String { return \"\"\"A\nB\"\"\" }\n";
    assert_eq!(
        source::check("triple.nexa", triple)
            .unwrap()
            .evaluate("value", &[])
            .unwrap(),
        Value::String("A\nB".to_owned())
    );

    let invalid_scalar = r#"nexa 0.3
module example.invalid_scalar
export fn value() -> String { return "\u{D800}" }
"#;
    assert_eq!(
        source::check("invalid_scalar.nexa", invalid_scalar)
            .unwrap_err()
            .diagnostic
            .code,
        "E_STRING_ESCAPE"
    );
}

#[test]
fn diagnostics_use_utf8_byte_spans() {
    let source_text = r#"nexa 0.3
module example.unicode
export fn greet(name is String) -> Unit { return "Zoë" + name }
"#;
    let error = source::check("unicode.nexa", source_text).unwrap_err();
    assert_eq!(error.diagnostic.code, "E_RETURN_TYPE");
    let span = error.diagnostic.span.unwrap();
    assert_eq!(span.source, "unicode.nexa");
    assert_eq!(span.start, source_text.find("\"Zoë\"").unwrap() as u64);
    assert_eq!(
        span.end,
        (source_text.find("name }").unwrap() + "name".len()) as u64
    );

    let parsed = source::parse("valid.nexa", GREET_SOURCE).unwrap();
    assert_eq!(parsed.module_name(), "local.hello.greet");
    assert_eq!(parsed.source_id(), "valid.nexa");
}

#[derive(Default)]
struct RecordingHost {
    calls: Vec<(String, Vec<Value>, SourceSpan)>,
}

impl Host for RecordingHost {
    fn invoke(
        &mut self,
        operation: &str,
        args: &[Value],
        call_site: &SourceSpan,
    ) -> nexa::diagnostic::Result<Value> {
        self.calls
            .push((operation.to_owned(), args.to_vec(), call_site.clone()));
        Ok(Value::Unit(()))
    }
}
