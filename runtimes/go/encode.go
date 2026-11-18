package tsurara

import (
	"fmt"
	"reflect"
	"strings"
)

// The Input methods generated functions call with their typed arguments. Values become ordered
// JSON values through encoding/json; a value that fails to encode is kept as EncodeError, and the
// call then fails with KindUnexpected without sending anything.

func (in *Input) value(v any) (any, bool) {
	value, err := ToValue(v)
	if err != nil {
		if in.EncodeError == nil {
			in.EncodeError = err
		}
		return nil, false
	}
	return value, true
}

// PathParam adds a path parameter.
func (in *Input) PathParam(name string, v any) {
	if value, ok := in.value(v); ok {
		in.Path = append(in.Path, Member{Key: name, Value: value})
	}
}

// QueryParam adds a query parameter.
func (in *Input) QueryParam(name string, v any) {
	if value, ok := in.value(v); ok {
		in.Query = append(in.Query, Member{Key: name, Value: value})
	}
}

// HeaderParam adds a header parameter.
func (in *Input) HeaderParam(name string, v any) {
	if value, ok := in.value(v); ok {
		in.Headers = append(in.Headers, Member{Key: name, Value: value})
	}
}

// CookieParam adds a cookie parameter.
func (in *Input) CookieParam(name string, v any) {
	if value, ok := in.value(v); ok {
		in.Cookies = append(in.Cookies, Member{Key: name, Value: value})
	}
}

// SetQueryString sets the whole query string of an in: querystring parameter.
func (in *Input) SetQueryString(v any) {
	if value, ok := in.value(v); ok {
		in.QueryString = value
		in.HasQS = true
	}
}

// SetBody sets a body that the media type serializes: JSON, form, text.
func (in *Input) SetBody(v any) {
	if value, ok := in.value(v); ok {
		in.Body = &Body{Value: value}
	}
}

// SetFile sets a binary body.
func (in *Input) SetFile(f File) {
	in.Body = &Body{File: &f}
}

// SetMediaType picks the request media type, for operations with several.
func (in *Input) SetMediaType(mediaType string) {
	in.ContentType = mediaType
}

var fileType = reflect.TypeFor[File]()

// SetMultipart sets a multipart body from a struct: one part per field in order, named by its JSON
// tag and left out as encoding/json would (omitempty, omitzero). Files (File, *File, []File) are
// file parts, one per file; other values are serialized by the encoding of their part.
func (in *Input) SetMultipart(v any) {
	rv := reflect.ValueOf(v)
	for rv.Kind() == reflect.Pointer {
		if rv.IsNil() {
			in.Body = &Body{Parts: []Part{}}
			return
		}
		rv = rv.Elem()
	}
	if rv.Kind() != reflect.Struct {
		if in.EncodeError == nil {
			in.EncodeError = fmt.Errorf("a multipart body must be a struct, not %s", rv.Type())
		}
		return
	}
	parts := []Part{}
	t := rv.Type()
	for i := range t.NumField() {
		field := t.Field(i)
		if !field.IsExported() {
			continue
		}
		name, opts, _ := strings.Cut(field.Tag.Get("json"), ",")
		if name == "-" && opts == "" {
			continue
		}
		if name == "" {
			name = field.Name
		}
		fv := rv.Field(i)
		if omitted(fv, opts) {
			continue
		}
		switch {
		case field.Type == fileType:
			f := fv.Interface().(File)
			parts = append(parts, Part{Name: name, File: &f})
		case field.Type.Kind() == reflect.Pointer && field.Type.Elem() == fileType:
			f := *fv.Interface().(*File)
			parts = append(parts, Part{Name: name, File: &f})
		case field.Type.Kind() == reflect.Slice && field.Type.Elem() == fileType:
			for _, f := range fv.Interface().([]File) {
				parts = append(parts, Part{Name: name, File: &f})
			}
		default:
			value, ok := in.value(fv.Interface())
			if !ok {
				return
			}
			parts = append(parts, Part{Name: name, Value: value})
		}
	}
	in.Body = &Body{Parts: parts}
}

type zeroer interface{ IsZero() bool }

// omitted reports a field that encoding/json leaves out with these tag options.
func omitted(v reflect.Value, opts string) bool {
	has := func(o string) bool {
		for _, x := range strings.Split(opts, ",") {
			if x == o {
				return true
			}
		}
		return false
	}
	if has("omitzero") {
		if z, ok := v.Interface().(zeroer); ok {
			if v.Kind() != reflect.Pointer || !v.IsNil() {
				return z.IsZero()
			}
		}
		if v.IsZero() {
			return true
		}
	}
	if has("omitempty") {
		switch v.Kind() {
		case reflect.Array, reflect.Map, reflect.Slice, reflect.String:
			return v.Len() == 0
		case reflect.Bool:
			return !v.Bool()
		case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
			return v.Int() == 0
		case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
			return v.Uint() == 0
		case reflect.Float32, reflect.Float64:
			return v.Float() == 0
		case reflect.Interface, reflect.Pointer:
			return v.IsNil()
		}
	}
	return false
}
