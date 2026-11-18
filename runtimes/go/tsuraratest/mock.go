// Package tsuraratest is a mock transport for tests of code that calls generated functions: it answers from handlers routed by operation, so tests need neither a server nor a mocking
// library. Routing follows spec/README.md (Mock transports), shared with the runtimes of the other
// languages.
//
//	m := tsuraratest.NewMockTransport(
//		tsuraratest.Operation(api.GetPetByIDDef(), func(r *tsuraratest.Request) (*tsuraratest.Response, error) {
//			return tsuraratest.JSON(200, api.Pet{Name: "Pochi"}), nil
//		}),
//	).WithBasePath("/v1")
//	c := tsurara.NewClient("https://api.test/v1", tsurara.WithTransport(m))
package tsuraratest

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"sync"

	tsurara "github.com/tsurara-openapi/tsurara/runtimes/go"
)

// Request is what a handler receives.
type Request struct {
	Method string
	URL    string
	// Params are the path parameters, percent-decoded.
	Params map[string]string
	Query  url.Values
	Header http.Header
	Body   []byte
}

// Response is what a handler returns.
type Response struct {
	Status int
	Header http.Header
	Body   []byte
}

// JSON is a response with a JSON body and Content-Type: application/json.
func JSON(status int, v any) *Response {
	body, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	return &Response{Status: status, Header: http.Header{"Content-Type": {"application/json"}}, Body: body}
}

// Text is a response with a text body.
func Text(status int, contentType, text string) *Response {
	return &Response{Status: status, Header: http.Header{"Content-Type": {contentType}}, Body: []byte(text)}
}

// Handler answers a request; an error fails the transport.
type Handler func(r *Request) (*Response, error)

type part struct {
	literal string
	param   string
}

// Route is a method and path template with its handler.
type Route struct {
	method    string
	segments  [][]part
	literals  int
	operation string
	handler   Handler
}

// Operation answers the operation's method and path template.
func Operation(def *tsurara.OperationDef, h Handler) Route {
	r := Path(def.Method, def.Path, h)
	r.operation = def.ID
	return r
}

// Path answers method and the path template path.
func Path(method, path string, h Handler) Route {
	r := Route{method: strings.ToUpper(method), handler: h}
	for _, segment := range strings.Split(path, "/") {
		var parts []part
		rest := segment
		for {
			start := strings.IndexByte(rest, '{')
			if start < 0 {
				break
			}
			end := strings.IndexByte(rest[start:], '}')
			if end < 0 {
				break
			}
			if start > 0 {
				parts = append(parts, part{literal: rest[:start]})
			}
			parts = append(parts, part{param: rest[start+1 : start+end]})
			rest = rest[start+end+1:]
		}
		if rest != "" || len(parts) == 0 {
			parts = append(parts, part{literal: rest})
		}
		literal := true
		for _, p := range parts {
			if p.param != "" {
				literal = false
			}
		}
		if literal {
			r.literals++
		}
		r.segments = append(r.segments, parts)
	}
	return r
}

// Call is a recorded request.
type Call struct {
	Method string
	URL    string
	// Route is the operation id of the route, when it was given an OperationDef.
	Route  string
	Params map[string]string
	Body   []byte
}

// MockTransport is a tsurara.Transport that answers from routes and records the calls.
type MockTransport struct {
	routes          []Route
	basePath        string
	recordUnmatched bool

	mu        sync.Mutex
	calls     []Call
	unmatched []Call
}

// NewMockTransport answers from routes.
func NewMockTransport(routes ...Route) *MockTransport {
	return &MockTransport{routes: routes}
}

// WithBasePath removes a base path (the path of the client's base URL) before routing.
func (m *MockTransport) WithBasePath(p string) *MockTransport {
	m.basePath = p
	return m
}

// RecordUnmatched answers unmatched requests with 501 and records them, instead of failing.
func (m *MockTransport) RecordUnmatched() *MockTransport {
	m.recordUnmatched = true
	return m
}

// Match returns the index of the route for a request path and its decoded parameters.
func (m *MockTransport) Match(method, path string) (int, map[string]string, bool) {
	path, _, _ = strings.Cut(path, "?")
	rest, ok := strings.CutPrefix(path, m.basePath)
	if !ok {
		return 0, nil, false
	}
	segments := strings.Split(rest, "/")
	best, bestLiterals := -1, -1
	var bestParams map[string]string
	for i, r := range m.routes {
		if !strings.EqualFold(r.method, method) || len(r.segments) != len(segments) {
			continue
		}
		params := map[string]string{}
		matched := true
		for j, parts := range r.segments {
			if !matchSegment(parts, segments[j], params) {
				matched = false
				break
			}
		}
		if matched && r.literals > bestLiterals {
			best, bestLiterals, bestParams = i, r.literals, params
		}
	}
	return best, bestParams, best >= 0
}

// matchSegment matches one segment; parameters are greedy, so the last match wins in a segment.
func matchSegment(parts []part, text string, params map[string]string) bool {
	if len(parts) == 0 {
		return text == ""
	}
	p := parts[0]
	if p.param == "" {
		rest, ok := strings.CutPrefix(text, p.literal)
		return ok && matchSegment(parts[1:], rest, params)
	}
	for end := len(text); end >= 1; end-- {
		tail := map[string]string{}
		if matchSegment(parts[1:], text[end:], tail) {
			value, err := url.PathUnescape(text[:end])
			if err != nil {
				value = text[:end]
			}
			params[p.param] = value
			for k, v := range tail {
				params[k] = v
			}
			return true
		}
	}
	return false
}

// Calls returns the calls that matched a route, in order.
func (m *MockTransport) Calls() []Call {
	m.mu.Lock()
	defer m.mu.Unlock()
	return append([]Call(nil), m.calls...)
}

// Unmatched returns the requests no route matched, with RecordUnmatched.
func (m *MockTransport) Unmatched() []Call {
	m.mu.Lock()
	defer m.mu.Unlock()
	return append([]Call(nil), m.unmatched...)
}

// Send implements tsurara.Transport.
func (m *MockTransport) Send(_ context.Context, r *tsurara.Request) (*tsurara.Response, error) {
	u, err := url.Parse(r.URL)
	if err != nil {
		return nil, err
	}
	path := u.EscapedPath()
	if path == "" {
		path = "/"
	}
	index, params, ok := m.Match(r.Method, path)
	call := Call{Method: r.Method, URL: r.URL, Params: params, Body: r.Body}
	if !ok {
		if m.recordUnmatched {
			m.mu.Lock()
			m.unmatched = append(m.unmatched, call)
			m.mu.Unlock()
			return &tsurara.Response{Status: 501, Header: http.Header{}}, nil
		}
		return nil, fmt.Errorf("tsurara mock: no route for %s %s", r.Method, path)
	}
	route := m.routes[index]
	call.Route = route.operation
	m.mu.Lock()
	m.calls = append(m.calls, call)
	m.mu.Unlock()
	resp, err := route.handler(&Request{
		Method: r.Method,
		URL:    r.URL,
		Params: params,
		Query:  u.Query(),
		Header: r.Header,
		Body:   r.Body,
	})
	if err != nil {
		return nil, fmt.Errorf("tsurara mock: the handler failed: %w", err)
	}
	header := resp.Header
	if header == nil {
		header = http.Header{}
	}
	return &tsurara.Response{Status: resp.Status, Header: header, Body: resp.Body}, nil
}
