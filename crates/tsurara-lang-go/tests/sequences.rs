//! Responses read item by item: Server-Sent Events, JSON Lines and JSON text sequences
//! are `*tsurara.Items[T]`, read with `tsurara.DoItems`.

use tsurara_lang_go::{GoOptions, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{Checker, GoChecker, repo_root};

fn files() -> Vec<(String, String)> {
    let text = std::fs::read(repo_root().join("corpus/edge-cases/event-streams.yaml")).unwrap();
    let set = DocumentSet::from_value(parse_document(&text).unwrap());
    let options = GoOptions {
        package: "api".into(),
        ..GoOptions::default()
    };
    generate(&set, &Options::default(), &options)
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

fn operations(files: &[(String, String)]) -> String {
    files
        .iter()
        .filter(|(p, _)| p.ends_with("_op_gen.go"))
        .map(|(_, c)| c.as_str())
        .collect()
}

#[test]
fn sequences_are_items() {
    let ops = operations(&files());
    assert!(
        ops.contains("func Stream(ctx context.Context, c *tsurara.Client, opts *StreamOptions) (*tsurara.Success[*tsurara.Items[StreamEvent]], error) {"),
        "{ops}"
    );
    assert!(
        ops.contains("return tsurara.DoItems[StreamEvent, any](ctx, c, streamDef(), in)"),
        "{ops}"
    );
    assert!(
        ops.contains(r#""parseAs":"events","jsonEvents":["note","noteDeleted"]"#),
        "{ops}"
    );
    assert!(ops.contains("*tsurara.Items[Note]"), "{ops}");
    // JSON next to a stream is read as JSON.
    assert!(ops.contains("(*tsurara.Success[Note], error)"), "{ops}");
}

const READ: &str = r#"package api

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	tsurara "github.com/tsurara-openapi/tsurara/runtimes/go"
)

func TestGeneratedFunctionsReadTypedItems(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/stream":
			w.Header().Set("Content-Type", "text/event-stream")
			_, _ = w.Write([]byte("event: note\ndata: {\"id\":\"1\",\"text\":\"hi\"}\nid: 5\n\nevent: noteDeleted\ndata: {\"id\":\"1\"}\n\nevent: unknown\ndata: x\n\n"))
		case "/mixed":
			w.Header().Set("Content-Type", "text/event-stream")
			_, _ = w.Write([]byte("event: pong\ndata: x\n\n"))
		case "/lines":
			w.Header().Set("Content-Type", "application/x-ndjson")
			_, _ = w.Write([]byte("{\"id\":\"3\"}\n{\"id\":\"4\"}\n"))
		default:
			w.Header().Set("Content-Type", "application/json-seq")
			_, _ = w.Write([]byte("\x1e{\"count\":1}\n"))
		}
	}))
	defer srv.Close()
	c := tsurara.NewClient(srv.URL)
	ctx := context.Background()

	s, err := Stream(ctx, c, nil)
	if err != nil {
		t.Fatal(err)
	}
	var seen []string
	for e, err := range s.Data.All() {
		if err != nil {
			t.Fatal(err)
		}
		seen = append(seen, describe(e))
	}
	want := []string{"note 1 hi 5", "noteDeleted 1", "other"}
	if len(seen) != len(want) {
		t.Fatalf("%v", seen)
	}
	for i := range want {
		if seen[i] != want[i] {
			t.Fatalf("%v", seen)
		}
	}

	l, err := Lines(ctx, c, nil)
	if err != nil {
		t.Fatal(err)
	}
	var ids []string
	for n, err := range l.Data.All() {
		if err != nil {
			t.Fatal(err)
		}
		ids = append(ids, n.ID)
	}
	if len(ids) != 2 || ids[0] != "3" || ids[1] != "4" {
		t.Fatalf("%v", ids)
	}

	m, err := Mixed(ctx, c, nil)
	if err != nil {
		t.Fatal(err)
	}
	for e, err := range m.Data.All() {
		if err != nil || e.MixedPingEvent == nil || e.MixedPingEvent.Event != MixedPingEventNamePong {
			t.Fatalf("%+v %v", e, err)
		}
	}

	q, err := Seq(ctx, c, nil)
	if err != nil {
		t.Fatal(err)
	}
	for item, err := range q.Data.All() {
		if err != nil || item.Count != 1 {
			t.Fatalf("%v %v", item, err)
		}
	}
}

// describe says which event e is, with its fields.
func describe(e StreamEvent) string {
	switch {
	case e.StreamNoteEvent != nil:
		n := e.StreamNoteEvent
		text := ""
		if n.Data.Text != nil {
			text = *n.Data.Text
		}
		return string(n.Event) + " " + n.Data.ID + " " + text + " " + *n.ID
	case e.StreamNoteDeletedEvent != nil:
		return string(e.StreamNoteDeletedEvent.Event) + " " + e.StreamNoteDeletedEvent.Data.ID
	}
	return "other"
}
"#;

#[test]
fn generated_functions_read_typed_items() {
    GoChecker {
        tests: vec![("read".into(), READ.into())],
        tags: Vec::new(),
    }
    .check(&files())
    .unwrap();
}
