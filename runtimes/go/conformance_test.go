package tsurara_test

// Runs the language-neutral conformance vectors in spec/conformance (design.md 18.9).

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"reflect"
	"strings"
	"testing"

	tsurara "github.com/tsurara-openapi/tsurara/runtimes/go"
)

func load(t *testing.T, name string) []tsurara.Object {
	t.Helper()
	data, err := os.ReadFile("../../spec/conformance/" + name)
	if err != nil {
		t.Fatal(err)
	}
	v, err := tsurara.ParseValue(data)
	if err != nil {
		t.Fatal(err)
	}
	var out []tsurara.Object
	for _, x := range v.([]any) {
		out = append(out, x.(tsurara.Object))
	}
	return out
}

func get(v any, keys ...string) any {
	for _, k := range keys {
		o, ok := v.(tsurara.Object)
		if !ok {
			return nil
		}
		v, _ = o.Get(k)
	}
	return v
}

func str(v any) string {
	s, _ := v.(string)
	return s
}

// mock answers every request with response and records the requests.
type mock struct {
	response *tsurara.Response
	sent     []*tsurara.Request
}

func (m *mock) Send(_ context.Context, r *tsurara.Request) (*tsurara.Response, error) {
	m.sent = append(m.sent, r)
	resp := *m.response
	return &resp, nil
}

func file(v any) *tsurara.File {
	if b := get(v, "$blob"); b != nil {
		return &tsurara.File{Content: []byte(str(get(b, "text"))), ContentType: str(get(b, "type"))}
	}
	if f := get(v, "$file"); f != nil {
		return &tsurara.File{Content: []byte(str(get(f, "text"))), Filename: str(get(f, "name")), ContentType: str(get(f, "type"))}
	}
	return nil
}

func members(v any) []tsurara.Member {
	o, _ := v.(tsurara.Object)
	return []tsurara.Member(o)
}

func input(v any) *tsurara.Input {
	in := &tsurara.Input{
		Path:        members(get(v, "path")),
		Query:       members(get(v, "query")),
		Headers:     members(get(v, "headers")),
		Cookies:     members(get(v, "cookies")),
		ContentType: str(get(v, "contentType")),
	}
	if o, ok := v.(tsurara.Object); ok {
		if qs, ok := o.Get("querystring"); ok {
			in.QueryString = qs
			in.HasQS = true
		}
		if b, ok := o.Get("body"); ok {
			if f := file(b); f != nil {
				in.Body = &tsurara.Body{File: f}
			} else if obj, ok := b.(tsurara.Object); ok && hasFile(obj) {
				var parts []tsurara.Part
				for _, m := range obj {
					if f := file(m.Value); f != nil {
						parts = append(parts, tsurara.Part{Name: m.Key, File: f})
					} else {
						parts = append(parts, tsurara.Part{Name: m.Key, Value: m.Value})
					}
				}
				in.Body = &tsurara.Body{Parts: parts}
			} else {
				in.Body = &tsurara.Body{Value: b}
			}
		}
	}
	return in
}

func hasFile(o tsurara.Object) bool {
	for _, m := range o {
		if file(m.Value) != nil {
			return true
		}
	}
	return false
}

func operation(t *testing.T, v any) *tsurara.OperationDef {
	t.Helper()
	return tsurara.ParseOperationDef(tsurara.JSONText(get(v, "operation")))
}

func client(response *tsurara.Response, credentials any) (*tsurara.Client, *mock) {
	m := &mock{response: response}
	c := tsurara.NewClient("https://api.test", tsurara.WithTransport(m), tsurara.WithAuth(func(s tsurara.SecurityUse, _ tsurara.OperationInfo) (string, bool) {
		v := get(credentials, s.Scheme)
		text, ok := v.(string)
		return text, ok
	}))
	return c, m
}

func kind[T any](f *tsurara.Failure[T]) string {
	if f == nil {
		return "success"
	}
	return string(f.Kind)
}

type part struct {
	name  string
	value any
}

// multipart parses a body into parts: a string for text parts, [filename, type, text] otherwise.
func multipart(contentType string, body []byte) []part {
	_, boundary, _ := strings.Cut(contentType, "boundary=")
	var out []part
	for _, p := range strings.Split(string(body), "--"+boundary) {
		trimmed := strings.TrimSpace(p)
		if trimmed == "" || trimmed == "--" {
			continue
		}
		head, content, _ := strings.Cut(strings.TrimPrefix(p, "\r\n"), "\r\n\r\n")
		content = strings.TrimSuffix(content, "\r\n")
		field := func(key string) (string, bool) {
			_, rest, ok := strings.Cut(head, key+"=\"")
			if !ok {
				return "", false
			}
			v, _, _ := strings.Cut(rest, "\"")
			return v, true
		}
		ctype := ""
		for _, l := range strings.Split(head, "\r\n") {
			if v, ok := strings.CutPrefix(l, "Content-Type: "); ok {
				ctype = v
			}
		}
		name, _ := field("name")
		filename, hasName := field("filename")
		if !hasName && ctype == "" {
			out = append(out, part{name, content})
			continue
		}
		var fn, ct any
		if hasName {
			fn = filename
		}
		if ctype != "" {
			ct = ctype
		}
		out = append(out, part{name, []any{fn, ct, content}})
	}
	return out
}

// plain turns an ordered value into encoding/json's values, for comparisons.
func plain(v any) any {
	var out any
	if err := json.Unmarshal([]byte(tsurara.JSONText(v)), &out); err != nil {
		panic(err)
	}
	return out
}

func TestRequestVectors(t *testing.T) {
	for _, v := range load(t, "requests.json") {
		name := str(get(v, "name"))
		t.Run(name, func(t *testing.T) {
			c, m := client(&tsurara.Response{Status: 204, Header: http.Header{}}, get(v, "credentials"))
			_, failure := c.DoRaw(context.Background(), operation(t, v), input(get(v, "input")))
			expect := get(v, "expect")
			if k := get(expect, "kind"); k != nil {
				if kind(failure) != k || len(m.sent) != 0 {
					t.Fatalf("expected %v without a request, got %s (%v)", k, kind(failure), failure)
				}
				return
			}
			if len(m.sent) == 0 {
				t.Fatalf("nothing sent: %v", failure)
			}
			req := m.sent[0]
			if req.URL != str(get(expect, "url")) {
				t.Errorf("url %s != %s", req.URL, str(get(expect, "url")))
			}
			if req.Method != str(get(expect, "method")) {
				t.Errorf("method %s", req.Method)
			}
			for _, h := range members(get(expect, "headers")) {
				got := req.Header.Get(h.Key)
				if values := req.Header[strings.ToLower(h.Key)]; len(values) > 0 {
					got = values[0]
				}
				if got != str(h.Value) {
					t.Errorf("header %s: %q != %q", h.Key, got, h.Value)
				}
			}
			want := get(expect, "body")
			switch {
			case want == nil:
				if len(req.Body) != 0 {
					t.Errorf("unexpected body %q", req.Body)
				}
			case get(want, "text") != nil:
				if string(req.Body) != str(get(want, "text")) {
					t.Errorf("body %q != %q", req.Body, get(want, "text"))
				}
			case get(want, "bytes") != nil:
				if string(req.Body) != str(get(want, "bytes", "text")) {
					t.Errorf("bytes %q", req.Body)
				}
			default:
				got := multipart(req.Header.Get("content-type"), req.Body)
				if ct := req.Header["content-type"]; len(ct) > 0 {
					got = multipart(ct[0], req.Body)
				}
				form := get(want, "form").([]any)
				if len(got) != len(form) {
					t.Fatalf("%d parts, expected %d: %v", len(got), len(form), got)
				}
				for i, w := range form {
					pair := w.([]any)
					wName, wValue := str(pair[0]), pair[1]
					var expected any
					switch {
					case get(wValue, "$file") != nil:
						f := get(wValue, "$file")
						expected = []any{get(f, "name"), get(f, "type"), get(f, "text")}
					case get(wValue, "$blob") != nil:
						b := get(wValue, "$blob")
						var first any
						if arr, ok := got[i].value.([]any); ok {
							first = arr[0]
						}
						expected = []any{first, get(b, "type"), get(b, "text")}
					default:
						expected = wValue
					}
					if got[i].name != wName || !reflect.DeepEqual(plain(got[i].value), plain(expected)) {
						t.Errorf("part %s: %v != %v", got[i].name, got[i].value, expected)
					}
				}
			}
		})
	}
}

func neutral(p tsurara.Payload) any {
	switch p.Kind {
	case tsurara.PayloadNone:
		return nil
	case tsurara.PayloadJSON:
		return plain(p.Value())
	case tsurara.PayloadText:
		return p.Text
	}
	return map[string]any{"$blob": map[string]any{"type": p.ContentType, "text": string(p.Bytes)}}
}

func TestResponseVectors(t *testing.T) {
	for _, v := range load(t, "responses.json") {
		name := str(get(v, "name"))
		t.Run(name, func(t *testing.T) {
			r := get(v, "response")
			status, _ := get(r, "status").(json.Number).Int64()
			header := http.Header{}
			for _, h := range members(get(r, "headers")) {
				header.Set(h.Key, str(h.Value))
			}
			resp := &tsurara.Response{Status: int(status), Header: header, Body: []byte(str(get(r, "body", "text")))}
			c, _ := client(resp, nil)
			success, failure := c.DoRaw(context.Background(), operation(t, v), nil)
			expect := get(v, "expect")
			if kind(failure) != str(get(expect, "kind")) {
				t.Fatalf("kind %s != %v (%v)", kind(failure), get(expect, "kind"), failure)
			}
			wantStatus := fmt.Sprint(plain(get(expect, "status")))
			switch {
			case success != nil:
				if fmt.Sprint(success.Status) != wantStatus || !reflect.DeepEqual(neutral(success.Data), plain(get(expect, "data"))) {
					t.Errorf("%d %v", success.Status, neutral(success.Data))
				}
			case failure.Kind == tsurara.KindHTTP:
				payload := failure.Raw
				if failure.Documented {
					payload = failure.Body
				}
				if fmt.Sprint(failure.Status) != wantStatus || failure.Documented != get(expect, "documented") || !reflect.DeepEqual(neutral(payload), plain(get(expect, "error"))) {
					t.Errorf("%d %v %v", failure.Status, failure.Documented, neutral(payload))
				}
			}
		})
	}
}

// chunks answers with a body that arrives one chunk per read.
type chunks struct {
	status int
	header http.Header
	parts  []string
}

func (c *chunks) Send(context.Context, *tsurara.Request) (*tsurara.Response, error) {
	panic("the client reads responses with SendStream")
}

func (c *chunks) SendStream(context.Context, *tsurara.Request) (*tsurara.StreamResponse, error) {
	return &tsurara.StreamResponse{Status: c.status, Header: c.header, Body: &chunkReader{parts: c.parts}}, nil
}

type chunkReader struct {
	parts  []string
	closed bool
}

func (r *chunkReader) Read(p []byte) (int, error) {
	if len(r.parts) == 0 {
		return 0, io.EOF
	}
	n := copy(p, r.parts[0])
	r.parts[0] = r.parts[0][n:]
	if r.parts[0] == "" {
		r.parts = r.parts[1:]
	}
	return n, nil
}

func (r *chunkReader) Close() error {
	r.closed = true
	return nil
}

func TestSequenceVectors(t *testing.T) {
	for _, v := range load(t, "sequences.json") {
		name := str(get(v, "name"))
		t.Run(name, func(t *testing.T) {
			r := get(v, "response")
			status, _ := get(r, "status").(json.Number).Int64()
			header := http.Header{}
			for _, h := range members(get(r, "headers")) {
				header.Set(h.Key, str(h.Value))
			}
			var parts []string
			for _, c := range get(r, "chunks").([]any) {
				parts = append(parts, str(c))
			}
			c := tsurara.NewClient("https://api.test", tsurara.WithTransport(&chunks{status: int(status), header: header, parts: parts}))
			success, err := tsurara.DoItems[any, any](context.Background(), c, operation(t, v), nil)
			expect := get(v, "expect")
			var failure tsurara.AnyFailure
			got := "success"
			if errors.As(err, &failure) {
				got = string(failure.Info().Kind)
			}
			if got != str(get(expect, "kind")) {
				t.Fatalf("kind %s != %v (%v)", got, get(expect, "kind"), err)
			}
			wantStatus := fmt.Sprint(plain(get(expect, "status")))
			if success == nil {
				f := err.(*tsurara.Failure[any])
				if fmt.Sprint(f.Status) != wantStatus || f.Documented != get(expect, "documented") || !reflect.DeepEqual(plain(f.Body), plain(get(expect, "error"))) {
					t.Errorf("%d %v %v", f.Status, f.Documented, f.Body)
				}
				return
			}
			items := []any{}
			failed := false
			for item, err := range success.Data.All() {
				if err != nil {
					failed = true
					break
				}
				items = append(items, plain(item))
			}
			wantFailed := get(expect, "error") == true
			if fmt.Sprint(success.Status) != wantStatus || !reflect.DeepEqual(items, plain(get(expect, "items"))) || failed != wantFailed {
				t.Errorf("%d %v failed=%v", success.Status, items, failed)
			}
		})
	}
}
