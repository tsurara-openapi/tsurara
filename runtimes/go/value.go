package tsurara

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"strconv"
	"strings"
	"unicode/utf8"
)

// Values the runtime serializes are JSON values with their object keys in order: nil, bool,
// json.Number, string, []any and Object. Generated functions pass typed values, which the Input
// methods turn into such values.

// Object is a JSON object that keeps the order of its keys.
type Object []Member

// Member is one key of an Object.
type Member struct {
	Key   string
	Value any
}

// Get returns the value of key.
func (o Object) Get(key string) (any, bool) {
	for _, m := range o {
		if m.Key == key {
			return m.Value, true
		}
	}
	return nil, false
}

// MarshalJSON writes the members in order.
func (o Object) MarshalJSON() ([]byte, error) {
	return []byte(JSONText(o)), nil
}

// ParseValue parses JSON into an ordered value.
func ParseValue(data []byte) (any, error) {
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.UseNumber()
	v, err := parseNext(dec)
	if err != nil {
		return nil, err
	}
	if _, err := dec.Token(); err == nil {
		return nil, fmt.Errorf("tsurara: trailing data after a JSON value")
	}
	return v, nil
}

func parseNext(dec *json.Decoder) (any, error) {
	tok, err := dec.Token()
	if err != nil {
		return nil, err
	}
	switch t := tok.(type) {
	case json.Delim:
		switch t {
		case '{':
			obj := Object{}
			for dec.More() {
				k, err := dec.Token()
				if err != nil {
					return nil, err
				}
				v, err := parseNext(dec)
				if err != nil {
					return nil, err
				}
				obj = append(obj, Member{Key: k.(string), Value: v})
			}
			if _, err := dec.Token(); err != nil {
				return nil, err
			}
			return obj, nil
		case '[':
			arr := []any{}
			for dec.More() {
				v, err := parseNext(dec)
				if err != nil {
					return nil, err
				}
				arr = append(arr, v)
			}
			if _, err := dec.Token(); err != nil {
				return nil, err
			}
			return arr, nil
		}
		return nil, fmt.Errorf("tsurara: unexpected %v", t)
	default:
		return t, nil
	}
}

// ToValue turns a Go value into an ordered JSON value through encoding/json.
func ToValue(v any) (any, error) {
	data, err := json.Marshal(v)
	if err != nil {
		return nil, err
	}
	return ParseValue(data)
}

// JSONText is JSON.stringify of an ordered value: no HTML escaping, the short escapes of JavaScript.
func JSONText(v any) string {
	var b strings.Builder
	writeJSON(&b, v)
	return b.String()
}

func writeJSON(b *strings.Builder, v any) {
	switch x := v.(type) {
	case nil:
		b.WriteString("null")
	case bool:
		b.WriteString(strconv.FormatBool(x))
	case json.Number:
		b.WriteString(numberText(x))
	case string:
		writeString(b, x)
	case []any:
		b.WriteByte('[')
		for i, item := range x {
			if i > 0 {
				b.WriteByte(',')
			}
			writeJSON(b, item)
		}
		b.WriteByte(']')
	case Object:
		b.WriteByte('{')
		for i, m := range x {
			if i > 0 {
				b.WriteByte(',')
			}
			writeString(b, m.Key)
			b.WriteByte(':')
			writeJSON(b, m.Value)
		}
		b.WriteByte('}')
	default:
		parsed, err := ToValue(x)
		if err != nil {
			b.WriteString("null")
			return
		}
		writeJSON(b, parsed)
	}
}

func writeString(b *strings.Builder, s string) {
	b.WriteByte('"')
	for _, r := range s {
		switch r {
		case '"':
			b.WriteString(`\"`)
		case '\\':
			b.WriteString(`\\`)
		case '\b':
			b.WriteString(`\b`)
		case '\f':
			b.WriteString(`\f`)
		case '\n':
			b.WriteString(`\n`)
		case '\r':
			b.WriteString(`\r`)
		case '\t':
			b.WriteString(`\t`)
		case utf8.RuneError:
			b.WriteString(`�`)
		default:
			if r < 0x20 {
				fmt.Fprintf(b, `\u%04x`, r)
			} else {
				b.WriteRune(r)
			}
		}
	}
	b.WriteByte('"')
}

// numberText prints a number as JavaScript does: integral values without a fraction, exponents
// from 1e21 and below 1e-6.
func numberText(n json.Number) string {
	s := string(n)
	if !strings.ContainsAny(s, ".eE") {
		return s
	}
	f, err := strconv.ParseFloat(s, 64)
	if err != nil {
		return s
	}
	abs := math.Abs(f)
	if abs >= 1e21 || (abs < 1e-6 && f != 0) {
		e := strconv.FormatFloat(f, 'e', -1, 64)
		mant, exp, _ := strings.Cut(e, "e")
		sign := exp[0]
		digits := strings.TrimLeft(exp[1:], "0")
		return mant + "e" + string(sign) + digits
	}
	return strconv.FormatFloat(f, 'f', -1, 64)
}

// Text is a value as text: strings as they are, numbers as JavaScript prints them, null empty,
// objects and arrays as JSON.
func Text(v any) string {
	switch x := v.(type) {
	case nil:
		return ""
	case string:
		return x
	case bool:
		return strconv.FormatBool(x)
	case json.Number:
		return numberText(x)
	default:
		return JSONText(x)
	}
}
