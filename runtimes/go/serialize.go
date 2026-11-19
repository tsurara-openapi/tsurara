package tsurara

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"strings"
)

// Serialization of parameters and request bodies (design.md 8.5), as in every other runtime.

const (
	componentSafe = "-_.!~*'()"
	reservedSafe  = "-_.!~*'();,/:@$"
)

func encodeWith(text, safe string) string {
	var b strings.Builder
	for i := 0; i < len(text); i++ {
		c := text[i]
		if ('a' <= c && c <= 'z') || ('A' <= c && c <= 'Z') || ('0' <= c && c <= '9') || strings.IndexByte(safe, c) >= 0 {
			b.WriteByte(c)
		} else {
			fmt.Fprintf(&b, "%%%02X", c)
		}
	}
	return b.String()
}

// encode is encodeURIComponent, or encodeURI without #?&=+ for allowReserved.
func encode(text string, allowReserved bool) string {
	if allowReserved {
		return encodeWith(text, reservedSafe)
	}
	return encodeWith(text, componentSafe)
}

func component(text string) string { return encodeWith(text, componentSafe) }

func contentValue(v any, contentType string) string {
	if strings.Contains(strings.ToLower(contentType), "json") {
		return JSONText(v)
	}
	return Text(v)
}

func explodeOr(def ParamDef, fallback bool) bool {
	if def.Explode != nil {
		return *def.Explode
	}
	return fallback
}

func serializePathParam(name string, value any, def ParamDef) string {
	style := def.Style
	if style == "" {
		style = "simple"
	}
	explode := explodeOr(def, false)
	enc := func(s string) string { return encode(s, def.AllowReserved) }
	if def.ContentType != "" {
		return enc(contentValue(value, def.ContentType))
	}
	prefix := map[string]string{"label": ".", "matrix": ";"}[style]
	switch v := value.(type) {
	case []any:
		items := make([]string, len(v))
		for i, x := range v {
			items[i] = enc(Text(x))
		}
		if style == "matrix" {
			if explode {
				var b strings.Builder
				for _, i := range items {
					b.WriteString(";" + name + "=" + i)
				}
				return b.String()
			}
			return ";" + name + "=" + strings.Join(items, ",")
		}
		sep := ","
		if explode && style == "label" {
			sep = "."
		}
		return prefix + strings.Join(items, sep)
	case Object:
		if explode {
			sep := map[string]string{"label": ".", "matrix": ";"}[style]
			if sep == "" {
				sep = ","
			}
			kv := make([]string, len(v))
			for i, m := range v {
				kv[i] = enc(m.Key) + "=" + enc(Text(m.Value))
			}
			return prefix + strings.Join(kv, sep)
		}
		var flat []string
		for _, m := range v {
			flat = append(flat, enc(m.Key), enc(Text(m.Value)))
		}
		if style == "matrix" {
			return ";" + name + "=" + strings.Join(flat, ",")
		}
		return prefix + strings.Join(flat, ",")
	default:
		text := enc(Text(value))
		if style == "matrix" {
			return ";" + name + "=" + text
		}
		return prefix + text
	}
}

// isDotOrEmpty reports a segment that URL parsing would remove or merge: empty, . or .. (also
// percent-encoded).
func isDotOrEmpty(segment string) bool {
	s := strings.ReplaceAll(strings.ReplaceAll(segment, "%2e", "."), "%2E", ".")
	return s == "" || s == "." || s == ".."
}

// expandPath embeds path parameters. It fails when the template does not start with / (appended to a
// base URL, it could change the host or port), and when a parameter is missing or would make a whole
// segment empty, . or .., which would change the path (spec/README.md).
func expandPath(template string, values []Member, defs Ordered[ParamDef]) (string, error) {
	if !strings.HasPrefix(template, "/") {
		return "", fmt.Errorf("the path %q does not start with /", template)
	}
	var segments []string
	for _, segment := range strings.Split(template, "/") {
		if !strings.Contains(segment, "{") {
			segments = append(segments, segment)
			continue
		}
		var expanded strings.Builder
		rest := segment
		for {
			start := strings.IndexByte(rest, '{')
			if start < 0 {
				break
			}
			expanded.WriteString(rest[:start])
			end := strings.IndexByte(rest[start:], '}')
			if end < 0 {
				end = len(rest) - 1
			} else {
				end += start
			}
			name := rest[start+1 : end]
			value, ok := Object(values).Get(name)
			if !ok {
				return "", fmt.Errorf("tsurara: the path parameter %s is missing", name)
			}
			def, _ := defs.Get(name)
			expanded.WriteString(serializePathParam(name, value, def))
			rest = rest[end+1:]
		}
		expanded.WriteString(rest)
		text := expanded.String()
		for _, part := range strings.Split(text, "/") {
			if isDotOrEmpty(part) {
				return "", fmt.Errorf("tsurara: a path parameter value makes the segment %q, which would change the path", text)
			}
		}
		segments = append(segments, text)
	}
	return strings.Join(segments, "/"), nil
}

func serializeQueryParam(name string, value any, def ParamDef) []string {
	style := def.Style
	if style == "" {
		style = "form"
	}
	explode := explodeOr(def, style == "form")
	enc := func(s string) string { return encode(s, def.AllowReserved) }
	key := component(name)
	if def.ContentType != "" {
		return []string{key + "=" + enc(contentValue(value, def.ContentType))}
	}
	if obj, ok := value.(Object); ok && style == "deepObject" {
		var out []string
		var walk func(prefix string, v any)
		walk = func(prefix string, v any) {
			switch x := v.(type) {
			case Object:
				for _, m := range x {
					walk(prefix+"["+m.Key+"]", m.Value)
				}
			case []any:
				for _, item := range x {
					out = append(out, component(prefix)+"="+enc(Text(item)))
				}
			default:
				out = append(out, component(prefix)+"="+enc(Text(v)))
			}
		}
		walk(name, obj)
		return out
	}
	sep := map[string]string{"spaceDelimited": "%20", "pipeDelimited": "|"}[style]
	if sep == "" {
		sep = ","
	}
	switch v := value.(type) {
	case []any:
		items := make([]string, len(v))
		for i, x := range v {
			items[i] = enc(Text(x))
		}
		if explode {
			out := make([]string, len(items))
			for i, item := range items {
				out[i] = key + "=" + item
			}
			return out
		}
		return []string{key + "=" + strings.Join(items, sep)}
	case Object:
		if explode {
			out := make([]string, len(v))
			for i, m := range v {
				out[i] = component(m.Key) + "=" + enc(Text(m.Value))
			}
			return out
		}
		var flat []string
		for _, m := range v {
			flat = append(flat, enc(m.Key), enc(Text(m.Value)))
		}
		return []string{key + "=" + strings.Join(flat, sep)}
	default:
		return []string{key + "=" + enc(Text(value))}
	}
}

func buildQuery(values []Member, defs Ordered[ParamDef]) string {
	var parts []string
	for _, m := range values {
		// RFC 6570, which the styles come from, treats null as undefined.
		if m.Value == nil {
			continue
		}
		def, _ := defs.Get(m.Key)
		parts = append(parts, serializeQueryParam(m.Key, m.Value, def)...)
	}
	return strings.Join(parts, "&")
}

// buildQueryString serializes an in: querystring parameter (3.2): the whole query string.
func buildQueryString(value any, contentType string) string {
	if obj, ok := value.(Object); ok && strings.Contains(strings.ToLower(contentType), "x-www-form-urlencoded") {
		explode := true
		def := ParamDef{Style: "form", Explode: &explode}
		var parts []string
		for _, m := range obj {
			if m.Value == nil {
				continue
			}
			parts = append(parts, serializeQueryParam(m.Key, m.Value, def)...)
		}
		return strings.Join(parts, "&")
	}
	return component(contentValue(value, contentType))
}

func serializeHeaderParam(value any, def ParamDef) string {
	if def.ContentType != "" {
		return contentValue(value, def.ContentType)
	}
	switch v := value.(type) {
	case []any:
		items := make([]string, len(v))
		for i, x := range v {
			items[i] = Text(x)
		}
		return strings.Join(items, ",")
	case Object:
		var parts []string
		for _, m := range v {
			if explodeOr(def, false) {
				parts = append(parts, m.Key+"="+Text(m.Value))
			} else {
				parts = append(parts, m.Key, Text(m.Value))
			}
		}
		return strings.Join(parts, ",")
	default:
		return Text(value)
	}
}

func buildCookie(values []Member, defs Ordered[ParamDef]) string {
	var parts []string
	for _, m := range values {
		name := m.Key
		def, _ := defs.Get(name)
		explode := explodeOr(def, true)
		if def.ContentType != "" {
			parts = append(parts, name+"="+component(contentValue(m.Value, def.ContentType)))
			continue
		}
		switch v := m.Value.(type) {
		case []any:
			items := make([]string, len(v))
			for i, x := range v {
				items[i] = component(Text(x))
			}
			if explode {
				for _, item := range items {
					parts = append(parts, name+"="+item)
				}
			} else {
				parts = append(parts, name+"="+strings.Join(items, ","))
			}
		case Object:
			if explode {
				for _, x := range v {
					parts = append(parts, x.Key+"="+component(Text(x.Value)))
				}
			} else {
				var flat []string
				for _, x := range v {
					flat = append(flat, x.Key, component(Text(x.Value)))
				}
				parts = append(parts, name+"="+strings.Join(flat, ","))
			}
		default:
			parts = append(parts, name+"="+component(Text(m.Value)))
		}
	}
	return strings.Join(parts, "; ")
}

func mediaTypeOf(contentType string) string {
	mt, _, _ := strings.Cut(contentType, ";")
	return strings.ToLower(strings.TrimSpace(mt))
}

func isJSONMedia(mt string) bool {
	return mt == "application/json" || strings.HasSuffix(mt, "+json")
}

// encoded is a request body with its Content-Type ("" for no header).
type encoded struct {
	content     []byte
	contentType string
}

func quoteParam(text string) string {
	return strings.NewReplacer(`\`, `\\`, `"`, `\"`, "\r", "%0D", "\n", "%0A").Replace(text)
}

func boundary() string {
	var b [16]byte
	_, _ = rand.Read(b[:])
	return "tsurara" + hex.EncodeToString(b[:])
}

func multipart(parts []Part, encoding Ordered[EncodingDef]) encoded {
	bnd := boundary()
	var out []byte
	part := func(name string, content []byte, contentType, filename string) {
		head := "--" + bnd + "\r\nContent-Disposition: form-data; name=\"" + quoteParam(name) + "\""
		if filename != "" {
			head += "; filename=\"" + quoteParam(filename) + "\""
		}
		head += "\r\n"
		if contentType != "" {
			head += "Content-Type: " + contentType + "\r\n"
		}
		head += "\r\n"
		out = append(out, head...)
		out = append(out, content...)
		out = append(out, "\r\n"...)
	}
	appendPart := func(p Part, encType string) {
		if p.File != nil {
			ct := p.File.ContentType
			if ct == "" {
				ct = "application/octet-stream"
			}
			part(p.Name, p.File.Content, ct, p.File.Filename)
			return
		}
		switch v := p.Value.(type) {
		case Object:
			// Without an encoding, a field with the JSON text, as the TypeScript runtime sends it
			// (FormData can only give a part a Content-Type by making it a file).
			part(p.Name, []byte(JSONText(v)), encType, "")
		case []any:
			if encType != "" {
				part(p.Name, []byte(JSONText(v)), encType, "")
			} else {
				part(p.Name, []byte(Text(v)), "", "")
			}
		default:
			if strings.Contains(strings.ToLower(encType), "json") {
				part(p.Name, []byte(JSONText(v)), encType, "")
			} else {
				part(p.Name, []byte(Text(v)), "", "")
			}
		}
	}
	for _, p := range parts {
		enc, _ := encoding.Get(p.Name)
		if items, ok := p.Value.([]any); ok && p.File == nil && enc.ContentType == "" {
			for _, item := range items {
				appendPart(Part{Name: p.Name, Value: item}, enc.ContentType)
			}
			continue
		}
		appendPart(p, enc.ContentType)
	}
	out = append(out, "--"+bnd+"--\r\n"...)
	return encoded{content: out, contentType: "multipart/form-data; boundary=" + bnd}
}

// encodeBody builds the request body for the requested media type, or the first one of the
// definition.
func encodeBody(body *Body, def *BodyDef, requested string) (encoded, error) {
	contentType := requested
	if contentType == "" && len(def.Contents) > 0 {
		contentType = def.Contents[0].Key
	}
	if contentType == "" {
		contentType = "application/json"
	}
	mt := mediaTypeOf(contentType)
	media, ok := def.Contents.Get(contentType)
	if !ok {
		media, _ = def.Contents.Get(mt)
	}
	if strings.HasPrefix(mt, "multipart/") {
		switch {
		case body.Parts != nil:
			return multipart(body.Parts, media.Encoding), nil
		default:
			obj, _ := body.Value.(Object)
			parts := make([]Part, len(obj))
			for i, m := range obj {
				parts[i] = Part{Name: m.Key, Value: m.Value}
			}
			return multipart(parts, media.Encoding), nil
		}
	}
	if mt == "application/x-www-form-urlencoded" {
		var parts []string
		if obj, ok := body.Value.(Object); ok {
			for _, m := range obj {
				e, _ := media.Encoding.Get(m.Key)
				style := e.Style
				if style == "" {
					style = "form"
				}
				explode := true
				if e.Explode != nil {
					explode = *e.Explode
				}
				d := ParamDef{Style: style, Explode: &explode, ContentType: e.ContentType}
				parts = append(parts, serializeQueryParam(m.Key, m.Value, d)...)
			}
		}
		text := strings.ReplaceAll(strings.Join(parts, "&"), "%20", "+")
		return encoded{content: []byte(text), contentType: contentType}, nil
	}
	if body.File != nil {
		if strings.Contains(mt, "*") {
			return encoded{content: body.File.Content, contentType: body.File.ContentType}, nil
		}
		if isJSONMedia(mt) {
			return encoded{}, fmt.Errorf("binary values cannot be sent as a %s body", mt)
		}
		return encoded{content: body.File.Content, contentType: contentType}, nil
	}
	if body.Parts != nil {
		return encoded{}, fmt.Errorf("parts cannot be sent as a %s body", mt)
	}
	if strings.HasPrefix(mt, "text/") {
		return encoded{content: []byte(Text(body.Value)), contentType: contentType}, nil
	}
	if strings.Contains(mt, "*") {
		return encoded{content: []byte(JSONText(body.Value)), contentType: "application/json"}, nil
	}
	if isJSONMedia(mt) {
		return encoded{content: []byte(JSONText(body.Value)), contentType: contentType}, nil
	}
	return encoded{}, fmt.Errorf("a %s body takes a binary value", mt)
}
