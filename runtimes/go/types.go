package tsurara

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"slices"
	"strings"
)

// Helpers that the MarshalJSON and UnmarshalJSON methods of generated types call
// (docs/languages/go.md): unions, additional properties and optional nullable fields.

// Nullable is an optional value that may also be null: Missing (the zero value), Null, or a value.
// With the omitzero tag option a missing value is left out.
type Nullable[T any] struct {
	value T
	state uint8
}

const (
	nullableMissing = iota
	nullableNull
	nullableValue
)

// Value returns a Nullable holding v.
func Value[T any](v T) Nullable[T] { return Nullable[T]{value: v, state: nullableValue} }

// Null returns a Nullable that is null.
func Null[T any]() Nullable[T] { return Nullable[T]{state: nullableNull} }

// Get returns the value and whether there is one.
func (n Nullable[T]) Get() (T, bool) { return n.value, n.state == nullableValue }

// IsNull reports an explicit null.
func (n Nullable[T]) IsNull() bool { return n.state == nullableNull }

// IsZero reports a missing value, for omitzero.
func (n Nullable[T]) IsZero() bool { return n.state == nullableMissing }

func (n Nullable[T]) MarshalJSON() ([]byte, error) {
	if n.state != nullableValue {
		return []byte("null"), nil
	}
	return json.Marshal(n.value)
}

func (n *Nullable[T]) UnmarshalJSON(data []byte) error {
	if bytes.Equal(bytes.TrimSpace(data), []byte("null")) {
		*n = Null[T]()
		return nil
	}
	var v T
	if err := json.Unmarshal(data, &v); err != nil {
		return err
	}
	*n = Value(v)
	return nil
}

// Decoder decodes a union member; strict decoding rejects unknown object fields, so that an object
// does not match every struct.
type Decoder func(data []byte, strict bool) error

func isNull(data []byte) bool { return bytes.Equal(bytes.TrimSpace(data), []byte("null")) }

// Into decodes a member into *p, allocating it. null is not a value of the member.
func Into[T any](p **T) Decoder {
	return func(data []byte, strict bool) error {
		if isNull(data) {
			return errors.New("null")
		}
		v := new(T)
		dec := json.NewDecoder(bytes.NewReader(data))
		if strict {
			dec.DisallowUnknownFields()
		}
		if err := dec.Decode(v); err != nil {
			return err
		}
		if dec.More() {
			return errors.New("trailing data")
		}
		*p = v
		return nil
	}
}

// IntoRaw keeps the JSON of a value (the Unknown member of an open union).
func IntoRaw(p *json.RawMessage) Decoder {
	return func(data []byte, _ bool) error {
		*p = slices.Clone(data)
		return nil
	}
}

// IntoNull sets *p for null, the null member of a union.
func IntoNull(p *bool) Decoder {
	return func(data []byte, _ bool) error {
		if !isNull(data) {
			return errors.New("not null")
		}
		*p = true
		return nil
	}
}

// NullMember is the null member of a union for MarshalUnion: null when set.
func NullMember(set bool) json.RawMessage {
	if set {
		return json.RawMessage("null")
	}
	return nil
}

// MarshalUnion encodes the first member that is set (a non-nil pointer or raw JSON).
func MarshalUnion(members ...any) ([]byte, error) {
	for _, m := range members {
		if raw, ok := m.(json.RawMessage); ok {
			if raw != nil {
				return raw, nil
			}
			continue
		}
		v := reflect.ValueOf(m)
		if v.Kind() == reflect.Pointer && v.IsNil() {
			continue
		}
		return json.Marshal(m)
	}
	return nil, errors.New("tsurara: no member of the union is set")
}

// TagCase is a member of a tagged union and the discriminator values that select it.
type TagCase struct {
	Values []string
	Decode Decoder
}

// UnmarshalTagged reads the discriminator property and decodes the whole object into the member it
// selects. A value no case lists goes to fallback (the default member, or the raw JSON of an open
// union); without one it is an error.
func UnmarshalTagged(data []byte, property string, cases []TagCase, fallback Decoder) error {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(data, &fields); err != nil || fields == nil {
		return fmt.Errorf("tsurara: a tagged union value must be an object: %s", truncate(data))
	}
	var tag string
	if raw, ok := fields[property]; ok {
		_ = json.Unmarshal(raw, &tag)
	}
	for _, c := range cases {
		if slices.Contains(c.Values, tag) {
			return c.Decode(data, false)
		}
	}
	if fallback != nil {
		return fallback(data, false)
	}
	return fmt.Errorf("tsurara: %q is not a listed value of the discriminator %q", tag, property)
}

// UnmarshalUntagged decodes into the first member that accepts the value, in order.
func UnmarshalUntagged(data []byte, members ...Decoder) error {
	for _, m := range members {
		if m(data, true) == nil {
			return nil
		}
	}
	return fmt.Errorf("tsurara: no member of the union accepts %s", truncate(data))
}

func truncate(data []byte) string {
	if len(data) > 60 {
		return string(data[:60]) + "..."
	}
	return string(data)
}

// MarshalWithExtra encodes known (the struct without its methods) and adds the additional
// properties after its fields, in key order.
func MarshalWithExtra[T any](known any, extra map[string]T) ([]byte, error) {
	data, err := json.Marshal(known)
	if err != nil || len(extra) == 0 {
		return data, err
	}
	taken := knownKeys(reflect.TypeOf(known))
	keys := make([]string, 0, len(extra))
	for k := range extra {
		if !taken[k] {
			keys = append(keys, k)
		}
	}
	slices.Sort(keys)
	var b bytes.Buffer
	b.Write(bytes.TrimSuffix(bytes.TrimSpace(data), []byte("}")))
	first := bytes.Equal(bytes.TrimSpace(data), []byte("{}"))
	for _, k := range keys {
		value, err := json.Marshal(extra[k])
		if err != nil {
			return nil, err
		}
		key, _ := json.Marshal(k)
		if !first {
			b.WriteByte(',')
		}
		first = false
		b.Write(key)
		b.WriteByte(':')
		b.Write(value)
	}
	b.WriteByte('}')
	return b.Bytes(), nil
}

// UnmarshalWithExtra decodes data into known (a pointer to the struct without its methods) and the
// properties its fields do not name into extra.
func UnmarshalWithExtra[T any](data []byte, known any, extra *map[string]T) error {
	if err := json.Unmarshal(data, known); err != nil {
		return err
	}
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(data, &fields); err != nil {
		return err
	}
	taken := knownKeys(reflect.TypeOf(known))
	*extra = nil
	for k, raw := range fields {
		if taken[k] {
			continue
		}
		var v T
		if err := json.Unmarshal(raw, &v); err != nil {
			return fmt.Errorf("tsurara: the additional property %q: %w", k, err)
		}
		if *extra == nil {
			*extra = map[string]T{}
		}
		(*extra)[k] = v
	}
	return nil
}

// knownKeys are the JSON names of a struct's fields.
func knownKeys(t reflect.Type) map[string]bool {
	for t.Kind() == reflect.Pointer {
		t = t.Elem()
	}
	keys := map[string]bool{}
	for i := range t.NumField() {
		f := t.Field(i)
		name, _, _ := strings.Cut(f.Tag.Get("json"), ",")
		switch {
		case name == "-":
		case name != "":
			keys[name] = true
		case f.IsExported():
			keys[f.Name] = true
		}
	}
	return keys
}
