package tsurara

import (
	"bytes"
	"encoding/json"
	"fmt"
)

// OperationDefVersion is the version of the OperationDef format this runtime reads.
const OperationDefVersion = 1

// OperationDef describes an operation (spec/operation-def.schema.json). Generated code embeds it as
// JSON and parses it once with ParseOperationDef.
type OperationDef struct {
	V         int                  `json:"v"`
	ID        string               `json:"id"`
	Method    string               `json:"method"`
	Path      string               `json:"path"`
	Tags      []string             `json:"tags"`
	Params    ParamsDef            `json:"params"`
	Body      *BodyDef             `json:"body"`
	Responses Ordered[ResponseDef] `json:"responses"`
	Security  [][]SecurityUse      `json:"security"`
}

// ParseOperationDef parses an OperationDef that generated code embeds. It panics on invalid JSON,
// which only a generator bug produces.
func ParseOperationDef(text string) *OperationDef {
	var def OperationDef
	if err := json.Unmarshal([]byte(text), &def); err != nil {
		panic("tsurara: invalid OperationDef: " + err.Error())
	}
	return &def
}

type ParamsDef struct {
	Path        Ordered[ParamDef] `json:"path"`
	Query       Ordered[ParamDef] `json:"query"`
	Headers     Ordered[ParamDef] `json:"headers"`
	Cookies     Ordered[ParamDef] `json:"cookies"`
	QueryString *QueryStringDef   `json:"querystring"`
}

type ParamDef struct {
	Style         string `json:"style"`
	Explode       *bool  `json:"explode"`
	AllowReserved bool   `json:"allowReserved"`
	ContentType   string `json:"contentType"`
}

type QueryStringDef struct {
	ContentType string `json:"contentType"`
}

type BodyDef struct {
	Required bool `json:"required"`
	// Contents per media type; the first is the default.
	Contents Ordered[MediaDef] `json:"contents"`
}

type MediaDef struct {
	Encoding Ordered[EncodingDef] `json:"encoding"`
}

type EncodingDef struct {
	ContentType string `json:"contentType"`
	Style       string `json:"style"`
	Explode     *bool  `json:"explode"`
}

// ParseAs says how a response body is read: json, text, blob, stream, none or auto, or item by
// item (events, jsonLines, jsonSeq; DoItems).
type ParseAs string

const (
	ParseJSON      ParseAs = "json"
	ParseText      ParseAs = "text"
	ParseBlob      ParseAs = "blob"
	ParseStream    ParseAs = "stream"
	ParseEvents    ParseAs = "events"
	ParseJSONLines ParseAs = "jsonLines"
	ParseJSONSeq   ParseAs = "jsonSeq"
	ParseNone      ParseAs = "none"
	ParseAuto      ParseAs = "auto"
)

type ResponseDef struct {
	ParseAs      ParseAs          `json:"parseAs"`
	ContentTypes Ordered[ParseAs] `json:"contentTypes"`
	// JSONEvents is, with events, true or the names of the events whose data is JSON.
	JSONEvents json.RawMessage `json:"jsonEvents"`
}

// SecurityUse is a security scheme an authentication alternative uses.
type SecurityUse struct {
	// Scheme is the name in components.securitySchemes.
	Scheme string `json:"scheme"`
	// Type is apiKey, http, oauth2, openIdConnect or mutualTLS.
	Type string `json:"type"`
	// In is where an apiKey goes: header, query or cookie.
	In string `json:"in"`
	// Name is the header, query or cookie name of an apiKey.
	Name string `json:"name"`
	// HTTPScheme is the Authorization scheme of an http scheme.
	HTTPScheme string   `json:"httpScheme"`
	Scopes     []string `json:"scopes"`
}

// OperationInfo is the operation of a request, for hooks and errors.
type OperationInfo struct {
	ID     string
	Method string
	Path   string
	Tags   []string
}

func (d *OperationDef) info() OperationInfo {
	return OperationInfo{ID: d.ID, Method: d.Method, Path: d.Path, Tags: d.Tags}
}

// Ordered is a JSON object read into its entries in order.
type Ordered[T any] []Entry[T]

type Entry[T any] struct {
	Key   string
	Value T
}

// Get returns the value of key.
func (o Ordered[T]) Get(key string) (T, bool) {
	for _, e := range o {
		if e.Key == key {
			return e.Value, true
		}
	}
	var zero T
	return zero, false
}

func (o *Ordered[T]) UnmarshalJSON(data []byte) error {
	dec := json.NewDecoder(bytes.NewReader(data))
	tok, err := dec.Token()
	if err != nil {
		return err
	}
	if tok == nil {
		*o = nil
		return nil
	}
	if d, ok := tok.(json.Delim); !ok || d != '{' {
		return fmt.Errorf("tsurara: expected an object, got %v", tok)
	}
	var out Ordered[T]
	for dec.More() {
		k, err := dec.Token()
		if err != nil {
			return err
		}
		var v T
		if err := dec.Decode(&v); err != nil {
			return err
		}
		out = append(out, Entry[T]{Key: k.(string), Value: v})
	}
	*o = out
	return nil
}
