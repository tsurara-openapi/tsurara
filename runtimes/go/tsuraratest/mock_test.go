package tsuraratest_test

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"strings"
	"testing"

	tsurara "github.com/tsurara-openapi/tsurara/runtimes/go"
	"github.com/tsurara-openapi/tsurara/runtimes/go/tsuraratest"
)

type vector struct {
	Name     string `json:"name"`
	BasePath string `json:"basePath"`
	Routes   []struct {
		Method string `json:"method"`
		Path   string `json:"path"`
	} `json:"routes"`
	Request struct {
		Method string `json:"method"`
		Path   string `json:"path"`
	} `json:"request"`
	Expect struct {
		Route  *int              `json:"route"`
		Params map[string]string `json:"params"`
	} `json:"expect"`
}

func TestRoutingVectors(t *testing.T) {
	text, err := os.ReadFile("../../../spec/conformance/mock-routing.json")
	if err != nil {
		t.Fatal(err)
	}
	var file struct {
		Vectors []vector `json:"vectors"`
	}
	if err := json.Unmarshal(text, &file); err != nil {
		t.Fatal(err)
	}
	for _, v := range file.Vectors {
		t.Run(v.Name, func(t *testing.T) {
			var routes []tsuraratest.Route
			for _, r := range v.Routes {
				routes = append(routes, tsuraratest.Path(r.Method, r.Path, func(*tsuraratest.Request) (*tsuraratest.Response, error) {
					return &tsuraratest.Response{Status: 204}, nil
				}))
			}
			m := tsuraratest.NewMockTransport(routes...).WithBasePath(v.BasePath)
			index, params, ok := m.Match(v.Request.Method, v.Request.Path)
			if v.Expect.Route == nil {
				if ok {
					t.Fatalf("matched route %d", index)
				}
				return
			}
			if !ok || index != *v.Expect.Route {
				t.Fatalf("got %d %v, want %d", index, ok, *v.Expect.Route)
			}
			if !reflect.DeepEqual(params, v.Expect.Params) {
				t.Fatalf("params %v, want %v", params, v.Expect.Params)
			}
		})
	}
}

var getPet = tsurara.ParseOperationDef(`{ "v": 1, "id": "getPet", "method": "GET", "path": "/pets/{petId}",
	"params": { "path": { "petId": {} } },
	"responses": { "200": { "parseAs": "json" }, "404": { "parseAs": "json" } } }`)

type pet struct {
	ID   int64  `json:"id"`
	Name string `json:"name"`
}

type problem struct {
	Message string `json:"message"`
}

func input(id int) *tsurara.Input {
	in := &tsurara.Input{}
	in.PathParam("petId", id)
	return in
}

func TestCallsThroughTheMock(t *testing.T) {
	m := tsuraratest.NewMockTransport(
		tsuraratest.Operation(getPet, func(r *tsuraratest.Request) (*tsuraratest.Response, error) {
			if r.Params["petId"] == "1" {
				return tsuraratest.JSON(200, pet{ID: 1, Name: "Pochi"}), nil
			}
			return tsuraratest.JSON(404, problem{Message: "no pet"}), nil
		}),
	).WithBasePath("/v1")
	c := tsurara.NewClient("https://api.test/v1", tsurara.WithTransport(m))
	ok, err := tsurara.Do[pet, problem](context.Background(), c, getPet, input(1))
	if err != nil || ok.Data.Name != "Pochi" {
		t.Fatalf("%v %v", ok, err)
	}
	_, err = tsurara.Do[pet, problem](context.Background(), c, getPet, input(9))
	var failure *tsurara.Failure[problem]
	if !errors.As(err, &failure) || failure.Kind != tsurara.KindHTTP || !failure.Documented || failure.Body.Message != "no pet" {
		t.Fatalf("%v", err)
	}
	calls := m.Calls()
	if len(calls) != 2 || calls[0].Route != "getPet" || calls[1].Params["petId"] != "9" {
		t.Fatalf("%+v", calls)
	}
}

func TestUnmatchedRequests(t *testing.T) {
	c := tsurara.NewClient("https://api.test", tsurara.WithTransport(tsuraratest.NewMockTransport()))
	_, err := tsurara.Do[pet, problem](context.Background(), c, getPet, input(1))
	var failure *tsurara.Failure[problem]
	if !errors.As(err, &failure) || failure.Kind != tsurara.KindNetwork || !strings.Contains(err.Error(), "GET /pets/1") {
		t.Fatalf("%v", err)
	}

	m := tsuraratest.NewMockTransport().RecordUnmatched()
	c = tsurara.NewClient("https://api.test", tsurara.WithTransport(m))
	_, err = tsurara.Do[pet, problem](context.Background(), c, getPet, input(1))
	if !errors.As(err, &failure) || failure.Status != 501 || len(m.Unmatched()) != 1 {
		t.Fatalf("%v %v", err, m.Unmatched())
	}
}

func TestFailingHandler(t *testing.T) {
	m := tsuraratest.NewMockTransport(tsuraratest.Path("GET", "/pets/{petId}", func(*tsuraratest.Request) (*tsuraratest.Response, error) {
		return nil, errors.New("connection reset")
	}))
	c := tsurara.NewClient("https://api.test", tsurara.WithTransport(m))
	_, err := tsurara.Do[pet, problem](context.Background(), c, getPet, input(1))
	var failure *tsurara.Failure[problem]
	if !errors.As(err, &failure) || failure.Kind != tsurara.KindNetwork {
		t.Fatalf("%v", err)
	}
}
