//! Lossless compression of the schemas advertised in MCP tools/list.
//!
//! The serde argument types remain authoritative. Only branches with identical
//! validation shapes apart from disjoint discriminator constants are grouped.

use serde_json::{Map, Value, json};

struct Member {
    tag: String,
    description: Option<String>,
    branch: Value,
}

struct Group {
    shape: Value,
    members: Vec<Member>,
}

pub(super) fn compact(schema: &Map<String, Value>, discriminator: &str) -> Map<String, Value> {
    // A new dialect, identifier scope, or reference into a oneOf branch could
    // make moving that branch change the meaning of an otherwise equal shape.
    if schema.get("$schema") != Some(&json!("https://json-schema.org/draft/2020-12/schema"))
        || schema.keys().any(|key| {
            !matches!(
                key.as_str(),
                "$schema" | "$defs" | "oneOf" | "properties" | "type"
            )
        })
        || has_scoped_or_positional_reference(&Value::Object(schema.clone()))
    {
        return schema.clone();
    }
    let Some(branches) = schema.get("oneOf").and_then(Value::as_array) else {
        return schema.clone();
    };
    let mut groups: Vec<Group> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for branch in branches {
        let Some(branch_object) = branch.as_object() else {
            return schema.clone();
        };
        if branch_object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "description" | "properties" | "required" | "type"
            )
        }) {
            return schema.clone();
        }
        let Some(properties) = branch.get("properties").and_then(Value::as_object) else {
            return schema.clone();
        };
        let Some(tag) = properties.get(discriminator).and_then(Value::as_object) else {
            return schema.clone();
        };
        if tag
            .keys()
            .any(|key| !matches!(key.as_str(), "const" | "type"))
            || tag.get("type") != Some(&json!("string"))
        {
            return schema.clone();
        }
        let Some(value) = tag.get("const").and_then(Value::as_str) else {
            return schema.clone();
        };
        if !seen.insert(value.to_owned())
            || !branch["required"]
                .as_array()
                .is_some_and(|required| required.contains(&json!(discriminator)))
        {
            return schema.clone();
        }
        let mut shape = branch.clone();
        let object = shape
            .as_object_mut()
            .expect("branch with properties is an object");
        if object
            .get("description")
            .is_some_and(|description| !description.is_string())
        {
            return schema.clone();
        }
        let description = object
            .remove("description")
            .and_then(|value| value.as_str().map(str::to_owned));
        object["properties"][discriminator]
            .as_object_mut()
            .expect("discriminator is an object")
            .remove("const");
        let member = Member {
            tag: value.to_owned(),
            description,
            branch: branch.clone(),
        };
        if let Some(group) = groups.iter_mut().find(|group| group.shape == shape) {
            group.members.push(member);
        } else {
            groups.push(Group {
                shape,
                members: vec![member],
            });
        }
    }

    let mut result = schema.clone();
    let mut grouped = Vec::with_capacity(groups.len());
    for Group { members, .. } in groups {
        if members.len() == 1 {
            grouped.push(members.into_iter().next().expect("one member").branch);
            continue;
        }
        let mut branch = members[0].branch.clone();
        let tags: Vec<_> = members.iter().map(|member| member.tag.clone()).collect();
        let tag_schema = branch["properties"][discriminator]
            .as_object_mut()
            .expect("discriminator is an object");
        tag_schema.remove("const");
        tag_schema.insert("enum".into(), json!(tags));
        let descriptions: Vec<_> = members
            .iter()
            .filter_map(|member| {
                member
                    .description
                    .as_ref()
                    .map(|description| format!("{}: {description}", member.tag))
            })
            .collect();
        if descriptions.is_empty() {
            branch
                .as_object_mut()
                .expect("branch object")
                .remove("description");
        } else {
            branch["description"] = json!(descriptions.join("; "));
        }
        grouped.push(branch);
    }
    result.insert("oneOf".into(), Value::Array(grouped));
    factor_repeated_properties(&mut result, discriminator);
    if serialized_len(&result) < serialized_len(schema) {
        result
    } else {
        schema.clone()
    }
}

fn factor_repeated_properties(schema: &mut Map<String, Value>, discriminator: &str) {
    let Some(branches) = schema.get("oneOf").and_then(Value::as_array) else {
        return;
    };
    let mut candidates: Vec<(String, Value)> = Vec::new();
    for branch in branches {
        if let Some(properties) = branch.get("properties").and_then(Value::as_object) {
            for (name, value) in properties {
                if name != discriminator
                    && is_simple_property_schema(value)
                    && !candidates.contains(&(name.clone(), value.clone()))
                {
                    candidates.push((name.clone(), value.clone()));
                }
            }
        }
    }
    for (name, value) in candidates {
        let mut trial = schema.clone();
        let defs = trial
            .entry("$defs")
            .or_insert_with(|| json!({}))
            .as_object_mut();
        let Some(defs) = defs else { continue };
        let mut suffix = 0;
        let definition = loop {
            let candidate = format!("compact_{name}_{suffix}");
            if !defs.contains_key(&candidate) {
                defs.insert(candidate.clone(), value.clone());
                break candidate;
            }
            suffix += 1;
        };
        let mut replacements = 0;
        for branch in trial["oneOf"].as_array_mut().expect("oneOf array") {
            if let Some(property) = branch
                .get_mut("properties")
                .and_then(Value::as_object_mut)
                .and_then(|properties| properties.get_mut(&name))
                && *property == value
            {
                *property = json!({"$ref":format!("#/$defs/{definition}")});
                replacements += 1;
            }
        }
        if replacements > 1 && serialized_len(&trial) < serialized_len(schema) {
            *schema = trial;
        }
    }
}

fn is_simple_property_schema(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        object.keys().all(|key| {
            matches!(
                key.as_str(),
                "type" | "default" | "format" | "minimum" | "maximum"
            )
        })
    })
}

fn has_scoped_or_positional_reference(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.keys().any(|key| {
                matches!(
                    key.as_str(),
                    "$id"
                        | "$anchor"
                        | "$dynamicAnchor"
                        | "$dynamicRef"
                        | "$recursiveAnchor"
                        | "$recursiveRef"
                )
            }) || object
                .get("$ref")
                .and_then(Value::as_str)
                .is_some_and(|reference| reference.starts_with("#/oneOf"))
                || object.values().any(has_scoped_or_positional_reference)
        }
        Value::Array(values) => values.iter().any(has_scoped_or_positional_reference),
        _ => false,
    }
}

fn serialized_len(schema: &Map<String, Value>) -> usize {
    serde_json::to_vec(schema).expect("schema serializes").len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfamiliar_schema_scopes_and_keywords_are_left_intact() {
        let original = json!({
            "$schema":"https://json-schema.org/draft/2020-12/schema",
            "type":"object",
            "oneOf":[
                {"type":"object","properties":{"view":{"const":"a","type":"string"}},"required":["view"]},
                {"type":"object","properties":{"view":{"const":"b","type":"string"}},"required":["view"]}
            ]
        });
        for (key, value) in [
            ("$id", json!("https://example.test/schema")),
            (
                "$schema",
                json!("https://json-schema.org/draft/2027-01/schema"),
            ),
            ("unevaluatedProperties", json!(false)),
        ] {
            let mut changed = original.clone();
            changed[key] = value;
            assert_eq!(
                compact(changed.as_object().unwrap(), "view"),
                *changed.as_object().unwrap()
            );
        }
        let mut changed = original.clone();
        changed["oneOf"][0]["properties"]["data"] = json!({"$id":"nested"});
        assert_eq!(
            compact(changed.as_object().unwrap(), "view"),
            *changed.as_object().unwrap()
        );
        let mut changed = original.clone();
        changed["$defs"] = json!({"branch":{"$ref":"#/oneOf/0"}});
        assert_eq!(
            compact(changed.as_object().unwrap(), "view"),
            *changed.as_object().unwrap()
        );
    }

    #[test]
    fn existing_definitions_are_never_replaced() {
        let mut schema = serde_json::to_value(
            super::super::Bridge::tool_router()
                .get("apr_inspect")
                .unwrap()
                .input_schema
                .as_ref(),
        )
        .unwrap();
        schema["$defs"]["compact_detail_0"] = json!({"const":"preserve"});
        let compacted = compact(schema.as_object().unwrap(), "view");
        assert_eq!(
            compacted["$defs"]["compact_detail_0"],
            json!({"const":"preserve"})
        );
    }
}
