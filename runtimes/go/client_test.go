package tsurara_test

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	tsurara "github.com/tsurara-openapi/tsurara/runtimes/go"
)

var def = tsurara.ParseOperationDef(`{ "v": 1, "id": "getPet", "method": "GET", "path": "/pets/{id}",
	"params": { "path": { "id": {} } },
	"responses": { "200": { "parseAs": "json" }, "404": { "parseAs": "json" } } }`)

func petInput() *tsurara.Input {
	in := &tsurara.Input{}
	in.PathParam("id", 1)
	return in
}

type pet struct {
	ID   int64  `json:"id"`
	Name string `json:"name"`
}

type problem struct {
	Message string `json:"message"`
}

// script answers with the responses in order (the last one repeats) after a delay.
type script struct {
	responses []*tsurara.Response
	calls     atomic.Int32
	delay     time.Duration
}

func (s *script) Send(ctx context.Context, _ *tsurara.Request) (*tsurara.Response, error) {
	n := int(s.calls.Add(1)) - 1
	select {
	case <-time.After(s.delay):
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	return s.responses[min(n, len(s.responses)-1)], nil
}

func jsonResponse(status int, body string) *tsurara.Response {
	return &tsurara.Response{Status: status, Header: http.Header{"Content-Type": {"application/json"}}, Body: []byte(body)}
}

func TestBodiesDecodeIntoTypes(t *testing.T) {
	s := &script{responses: []*tsurara.Response{jsonResponse(200, `{"id":1,"name":"a"}`), jsonResponse(404, `{"message":"no"}`), jsonResponse(500, `boom`), jsonResponse(200, `{"id":"x"}`)}}
	c := tsurara.NewClient("https://api.test", tsurara.WithTransport(s))
	ctx := context.Background()
	ok, err := tsurara.Do[pet, problem](ctx, c, def, petInput())
	if err != nil || ok.Data != (pet{1, "a"}) || ok.Status != 200 {
		t.Fatalf("%v %v", ok, err)
	}
	_, err = tsurara.Do[pet, problem](ctx, c, def, petInput())
	var f *tsurara.Failure[problem]
	if !errors.As(err, &f) || f.Kind != tsurara.KindHTTP || !f.Documented || f.Body.Message != "no" {
		t.Fatalf("%#v", err)
	}
	_, err = tsurara.Do[pet, problem](ctx, c, def, petInput())
	if !errors.As(err, &f) || f.Documented || f.Status != 500 || f.Raw.Text != "boom" {
		t.Fatalf("%#v", err)
	}
	_, err = tsurara.Do[pet, problem](ctx, c, def, petInput())
	if !errors.As(err, &f) || f.Kind != tsurara.KindParse {
		t.Fatalf("%#v", err)
	}
}

func TestRetriesHonorRetryAfterAndStopAtTheTimeout(t *testing.T) {
	busy := &tsurara.Response{Status: 503, Header: http.Header{"Retry-After": {"0"}}}
	s := &script{responses: []*tsurara.Response{busy, busy, jsonResponse(200, `{"id":1,"name":"a"}`)}}
	c := tsurara.NewClient("https://api.test", tsurara.WithTransport(s), tsurara.WithRetry(tsurara.DefaultRetry()))
	if _, err := tsurara.Do[pet, problem](context.Background(), c, def, petInput()); err != nil {
		t.Fatal(err)
	}
	if s.calls.Load() != 3 {
		t.Fatalf("%d calls", s.calls.Load())
	}
	// A Retry-After beyond the timeout is not waited for.
	late := &tsurara.Response{Status: 503, Header: http.Header{"Retry-After": {"60"}}}
	s = &script{responses: []*tsurara.Response{late}}
	c = tsurara.NewClient("https://api.test", tsurara.WithTransport(s), tsurara.WithRetry(tsurara.DefaultRetry()), tsurara.WithTimeout(time.Second))
	_, err := tsurara.Do[pet, problem](context.Background(), c, def, petInput())
	var f *tsurara.Failure[problem]
	if !errors.As(err, &f) || f.Status != 503 || s.calls.Load() != 1 {
		t.Fatalf("%v, %d calls", err, s.calls.Load())
	}
	at := time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)
	if d, ok := tsurara.RetryAfter(http.Header{"Retry-After": {"Thu, 01 Jan 2026 00:00:05 GMT"}}, at); !ok || d != 5*time.Second {
		t.Fatalf("%v %v", d, ok)
	}
}

func TestTimeoutsCancellationAndHooks(t *testing.T) {
	s := &script{responses: []*tsurara.Response{jsonResponse(200, `{}`)}, delay: time.Second}
	var kinds []tsurara.Kind
	c := tsurara.NewClient("https://api.test", tsurara.WithTransport(s), tsurara.OnResult(func(_ tsurara.OperationInfo, k tsurara.Kind) { kinds = append(kinds, k) }))
	in := petInput()
	in.Timeout = 20 * time.Millisecond
	_, err := tsurara.Do[pet, problem](context.Background(), c, def, in)
	var f *tsurara.Failure[problem]
	if !errors.As(err, &f) || f.Kind != tsurara.KindTimeout {
		t.Fatalf("%v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	go func() {
		time.Sleep(20 * time.Millisecond)
		cancel()
	}()
	_, err = tsurara.Do[pet, problem](ctx, c, def, petInput())
	if !errors.As(err, &f) || f.Kind != tsurara.KindAborted || !errors.Is(err, context.Canceled) {
		t.Fatalf("%v", err)
	}
	if len(kinds) != 2 || kinds[0] != tsurara.KindTimeout || kinds[1] != tsurara.KindAborted {
		t.Fatalf("%v", kinds)
	}
	in = petInput()
	in.QueryParam("bad", func() {})
	_, err = tsurara.Do[pet, problem](context.Background(), c, def, in)
	if !errors.As(err, &f) || f.Kind != tsurara.KindUnexpected {
		t.Fatalf("%v", err)
	}
}

type upload struct {
	Name   string         `json:"name"`
	File   tsurara.File   `json:"file"`
	Thumb  *tsurara.File  `json:"thumb,omitzero"`
	Note   *string        `json:"note,omitzero"`
	Extras []tsurara.File `json:"extras,omitzero"`
	Tags   []string       `json:"tags"`
	Hidden string         `json:"-"`
}

func TestTheHTTPTransportTalksToAServer(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		w.Header().Set("Content-Type", "text/plain")
		_, _ = w.Write([]byte(r.Method + " " + r.URL.RequestURI() + " " + r.Header.Get("Content-Type") + "\n" + string(body)))
	}))
	defer server.Close()
	echo := tsurara.ParseOperationDef(`{ "v": 1, "id": "echo", "method": "POST", "path": "/echo/{id}",
		"body": { "contents": { "multipart/form-data": {} } },
		"responses": { "200": { "parseAs": "text" } } }`)
	c := tsurara.NewClient(server.URL, tsurara.WithHTTPClient(server.Client()))
	in := &tsurara.Input{}
	in.PathParam("id", "a b")
	in.QueryParam("q", []string{"x", "y"})
	note := "hi"
	in.SetMultipart(upload{Name: "n", File: tsurara.File{Content: []byte("hello"), Filename: "a.txt"}, Note: &note, Extras: []tsurara.File{{Content: []byte("e1")}, {Content: []byte("e2")}}, Tags: []string{"t"}, Hidden: "h"})
	res, err := tsurara.Do[string, any](context.Background(), c, echo, in)
	if err != nil {
		t.Fatal(err)
	}
	text := res.Data
	for _, want := range []string{"POST /echo/a%20b?q=x&q=y multipart/form-data; boundary=", `name="file"; filename="a.txt"`, "hello", `name="note"`, "e1", "e2", `name="tags"`} {
		if !strings.Contains(text, want) {
			t.Errorf("missing %q in\n%s", want, text)
		}
	}
	for _, unwanted := range []string{`name="thumb"`, `name="Hidden"`, "\nh\r"} {
		if strings.Contains(text, unwanted) {
			t.Errorf("unexpected %q in\n%s", unwanted, text)
		}
	}
}

func TestFailuresOfEveryOperationAreReadWithOneInterface(t *testing.T) {
	s := &script{responses: []*tsurara.Response{jsonResponse(404, `{"message":"no"}`)}}
	c := tsurara.NewClient("https://api.test", tsurara.WithTransport(s))
	_, typed := tsurara.Do[pet, problem](context.Background(), c, def, petInput())
	_, untyped := tsurara.Do[pet, any](context.Background(), c, def, petInput())
	for _, err := range []error{typed, untyped} {
		var f tsurara.AnyFailure
		if !errors.As(err, &f) {
			t.Fatalf("%v", err)
		}
		info := f.Info()
		if info.Kind != tsurara.KindHTTP || info.Status != 404 || !info.Documented || info.Operation.ID != "getPet" {
			t.Fatalf("%+v", info)
		}
	}
}

func TestEventsArriveAsWrittenAndOutliveTheTimeout(t *testing.T) {
	second := make(chan struct{})
	closed := make(chan struct{})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		fmt.Fprint(w, "event: tick\ndata: {\"n\":1}\nid: 1\n\n")
		w.(http.Flusher).Flush()
		// The second event comes after the client's timeout, once the client has read the first.
		<-second
		time.Sleep(150 * time.Millisecond)
		fmt.Fprint(w, "event: tick\ndata: {\"n\":2}\n\n")
		w.(http.Flusher).Flush()
		<-r.Context().Done()
		close(closed)
	}))
	defer srv.Close()
	ticks := tsurara.ParseOperationDef(`{"v":1,"id":"ticks","method":"GET","path":"/ticks","responses":{"200":{"parseAs":"events","jsonEvents":true}}}`)
	c := tsurara.NewClient(srv.URL, tsurara.WithTimeout(100*time.Millisecond))
	type tick struct {
		Event string `json:"event"`
		Data  struct {
			N int `json:"n"`
		} `json:"data"`
		ID *string `json:"id"`
	}
	s, err := tsurara.DoItems[tick, any](context.Background(), c, ticks, nil)
	if err != nil {
		t.Fatal(err)
	}
	var got []int
	for e, err := range s.Data.All() {
		if err != nil {
			t.Fatal(err)
		}
		got = append(got, e.Data.N)
		if len(got) == 1 {
			if e.ID == nil || *e.ID != "1" {
				t.Fatalf("id %v", e.ID)
			}
			close(second)
			continue
		}
		break
	}
	if !reflect.DeepEqual(got, []int{1, 2}) {
		t.Fatalf("%v", got)
	}
	select {
	case <-closed:
	case <-time.After(5 * time.Second):
		t.Fatal("leaving the loop did not close the connection")
	}
}

func TestRedirectsAreFollowedWithinTheOriginOnly(t *testing.T) {
	var leaked []string
	other := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		leaked = append(leaked, r.Header.Get("X-Api-Key"))
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"id":9,"name":"Other"}`))
	}))
	defer other.Close()
	var server *httptest.Server
	server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/pets/1":
			http.Redirect(w, r, "/pets/2", http.StatusFound)
		case "/pets/2":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"id":2,"name":"` + r.Header.Get("X-Api-Key") + `"}`))
		case "/pets/3":
			http.Redirect(w, r, other.URL+"/pets/9", http.StatusTemporaryRedirect)
		}
	}))
	defer server.Close()
	c := tsurara.NewClient(server.URL, tsurara.WithHeader("X-Api-Key", "secret"))

	res, err := tsurara.Do[pet, problem](context.Background(), c, def, petInput())
	if err != nil || res.Data.Name != "secret" {
		t.Fatalf("a redirect within the origin is followed with the headers: %+v %v", res, err)
	}

	in := &tsurara.Input{}
	in.PathParam("id", 3)
	_, err = tsurara.Do[pet, problem](context.Background(), c, def, in)
	var f tsurara.AnyFailure
	if !errors.As(err, &f) || f.Info().Kind != tsurara.KindHTTP || f.Info().Status != http.StatusTemporaryRedirect {
		t.Fatalf("a redirect to another origin is the result: %v", err)
	}
	if len(leaked) != 0 {
		t.Fatalf("the other origin received %q", leaked)
	}
}
