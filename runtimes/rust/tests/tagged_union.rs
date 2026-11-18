//! `tagged_union!`: discriminated unions whose members keep their tag field.

use tsurara_client::serde::{Deserialize, Serialize};
use tsurara_client::serde_json::{self, Value, json};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(crate = "tsurara_client::serde")]
struct Cat {
    #[serde(rename = "petType")]
    pet_type: String,
    indoor: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(crate = "tsurara_client::serde")]
struct Dog {
    #[serde(rename = "petType")]
    pet_type: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Pet {
    Cat(Cat),
    Dog(Dog),
    Unknown(Value),
}

tsurara_client::tagged_union! {
    Pet, tag "petType" {
        Cat: ["cat"],
        Dog: ["dog", "doggo"],
    }
    unknown Unknown;
}

#[derive(Debug, Clone, PartialEq)]
enum Closed {
    Cat(Cat),
}

tsurara_client::tagged_union! {
    Closed, tag "petType" {
        Cat: ["cat"],
    }
}

#[test]
fn members_are_selected_by_the_tag_and_keep_it() {
    let cat: Pet = serde_json::from_value(json!({ "petType": "cat", "indoor": true })).unwrap();
    assert_eq!(
        cat,
        Pet::Cat(Cat {
            pet_type: "cat".into(),
            indoor: true
        })
    );
    let dog: Pet = serde_json::from_value(json!({ "petType": "doggo" })).unwrap();
    assert!(matches!(dog, Pet::Dog(_)));
    assert_eq!(
        serde_json::to_value(&cat).unwrap(),
        json!({ "petType": "cat", "indoor": true })
    );
}

#[test]
fn unlisted_tags_are_kept_or_rejected() {
    let bird: Pet = serde_json::from_value(json!({ "petType": "bird", "wings": 2 })).unwrap();
    assert_eq!(bird, Pet::Unknown(json!({ "petType": "bird", "wings": 2 })));
    assert_eq!(
        serde_json::to_value(&bird).unwrap(),
        json!({ "petType": "bird", "wings": 2 })
    );
    assert!(serde_json::from_value::<Closed>(json!({ "petType": "bird" })).is_err());
    assert!(serde_json::from_value::<Closed>(json!({ "indoor": true })).is_err());
}

#[derive(Debug, Clone, PartialEq)]
enum WithDefault {
    Cat(Cat),
    Dog(Dog),
}

tsurara_client::tagged_union! {
    WithDefault, tag "petType" {
        Cat: ["cat"],
    }
    default Dog;
}

#[test]
fn unlisted_tags_select_the_default_member() {
    let dog: WithDefault = serde_json::from_value(json!({ "petType": "wolf" })).unwrap();
    assert_eq!(
        dog,
        WithDefault::Dog(Dog {
            pet_type: "wolf".into()
        })
    );
    assert_eq!(
        serde_json::to_value(&dog).unwrap(),
        json!({ "petType": "wolf" })
    );
    let cat: WithDefault =
        serde_json::from_value(json!({ "petType": "cat", "indoor": false })).unwrap();
    assert!(matches!(cat, WithDefault::Cat(_)));
    assert!(serde_json::from_value::<WithDefault>(json!({ "kind": "x" })).is_err());
}
