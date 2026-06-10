//! Integration tests: playbook parsing.

use ansiblers_it::fixture_path;
use ansiblers_parser::{parse_playbook, TaskNode};
use rstest::rstest;

#[rstest]
#[case("simple_shell.yml")]
#[case("with_variables.yml")]
#[case("with_blocks.yml")]
#[case("with_loops.yml")]
fn test_parse_fixture_playbook(#[case] filename: &str) {
    let path = fixture_path(&format!("playbooks/{filename}"));
    let pb = parse_playbook(&path)
        .unwrap_or_else(|e| panic!("failed to parse {filename}: {e}"));
    assert!(
        !pb.plays.is_empty(),
        "{filename} should have at least one play"
    );
}

#[test]
fn test_snapshot_simple_shell_ast() {
    let path = fixture_path("playbooks/simple_shell.yml");
    let pb = parse_playbook(&path).unwrap();
    insta::assert_json_snapshot!(pb);
}

#[test]
fn test_parse_blocks_structure() {
    let path = fixture_path("playbooks/with_blocks.yml");
    let pb = parse_playbook(&path).unwrap();
    let play = &pb.plays[0];
    let first_task = &play.tasks[0];
    assert!(
        matches!(first_task, TaskNode::Block(_)),
        "first task node should be a block"
    );
}

#[test]
fn test_parse_variables_in_play() {
    let path = fixture_path("playbooks/with_variables.yml");
    let pb = parse_playbook(&path).unwrap();
    let play = &pb.plays[0];
    assert!(
        play.vars.contains_key("greeting"),
        "play should have 'greeting' var"
    );
    assert_eq!(
        play.vars["greeting"].as_str().unwrap(),
        "Hello"
    );
}
