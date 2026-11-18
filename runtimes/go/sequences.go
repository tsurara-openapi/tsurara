package tsurara

// Responses read item by item: Server-Sent Events, JSON Lines and JSON text sequences (design.md
// 8.5). Items reads the body as it arrives and decodes each complete item.

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"iter"
	"strconv"
	"strings"
	"sync"
)

// sequenceKind says how the items of a body are separated.
type sequenceKind int

const (
	seqEvents sequenceKind = iota
	seqJSONLines
	seqJSONSeq
)

// stream is a body read item by item, before its items have a type.
type stream struct {
	body       io.ReadCloser
	kind       sequenceKind
	jsonEvents json.RawMessage
	// stop releases the request (its context); called once the body is closed.
	stop func()
}

// Items are the items of a response, read as they arrive: events ({"event", "data", "id"?,
// "retry"?} with JSON data parsed) or JSON values, each decoded as T. Read them once with All, and
// Close them when not reading to the end.
type Items[T any] struct {
	s    *stream
	once sync.Once
}

// Close closes the response. Reading to the end, or leaving the loop over All, closes it too.
func (it *Items[T]) Close() error {
	var err error
	it.once.Do(func() {
		err = it.s.body.Close()
		if it.s.stop != nil {
			it.s.stop()
		}
	})
	return err
}

// All yields the items in order. Reading stops at the first error: a broken connection, or an
// item that is not valid JSON or does not match T.
func (it *Items[T]) All() iter.Seq2[T, error] {
	return func(yield func(T, error) bool) {
		defer it.Close()
		p := newParser(it.s.kind, it.s.jsonEvents)
		buf := make([]byte, 32*1024)
		emit := func(items [][]byte, err error) bool {
			for _, item := range items {
				v, derr := decodeItem[T](item)
				if derr != nil {
					var zero T
					yield(zero, derr)
					return false
				}
				if !yield(v, nil) {
					return false
				}
			}
			if err != nil {
				var zero T
				yield(zero, err)
				return false
			}
			return true
		}
		for {
			n, rerr := it.s.body.Read(buf)
			if n > 0 && !emit(p.feed(buf[:n], false)) {
				return
			}
			if errors.Is(rerr, io.EOF) {
				emit(p.feed(nil, true))
				return
			}
			if rerr != nil {
				var zero T
				yield(zero, fmt.Errorf("tsurara: reading the stream failed: %w", rerr))
				return
			}
		}
	}
}

// decodeItem reads the JSON of one item into T; into any it is an ordered value.
func decodeItem[T any](item []byte) (T, error) {
	var v T
	if a, ok := any(&v).(*any); ok {
		parsed, err := ParseValue(item)
		*a = parsed
		return v, err
	}
	if err := json.Unmarshal(item, &v); err != nil {
		return v, fmt.Errorf("tsurara: an item does not match its type: %w", err)
	}
	return v, nil
}

// parser splits bytes into items, as JSON texts. Lines and records are split on bytes (CR, LF and
// U+001E never occur inside a multi-byte UTF-8 character).
type parser struct {
	kind       sequenceKind
	allJSON    bool
	jsonNames  []string
	buffer     []byte
	started    bool
	bomChecked bool
	count      int
	event      string
	data       []string
	id         *string
	retry      *int64
}

func newParser(kind sequenceKind, jsonEvents json.RawMessage) *parser {
	p := &parser{kind: kind, started: kind != seqJSONSeq}
	var all bool
	if json.Unmarshal(jsonEvents, &all) == nil {
		p.allJSON = all
	} else {
		_ = json.Unmarshal(jsonEvents, &p.jsonNames)
	}
	return p
}

func (p *parser) feed(chunk []byte, last bool) ([][]byte, error) {
	p.buffer = append(p.buffer, chunk...)
	if !p.bomChecked {
		bom := []byte("\xEF\xBB\xBF")
		if len(p.buffer) < 3 && !last && bytes.HasPrefix(bom, p.buffer) {
			return nil, nil
		}
		p.buffer = bytes.TrimPrefix(p.buffer, bom)
		p.bomChecked = true
	}
	var parts [][]byte
	start := 0
	for i := 0; i < len(p.buffer); {
		b := p.buffer[i]
		end := -1
		switch {
		case p.kind == seqJSONSeq:
			if b == 0x1E {
				end = i + 1
			}
		case b == '\n':
			end = i + 1
		case b == '\r' && i+1 == len(p.buffer) && !last:
			// A CR at the end may be the first half of a CRLF, so it waits for what follows.
			i = len(p.buffer)
			continue
		case b == '\r':
			end = i + 1
			if i+1 < len(p.buffer) && p.buffer[i+1] == '\n' {
				end = i + 2
			}
		}
		if end < 0 {
			i++
			continue
		}
		parts = append(parts, bytes.Clone(p.buffer[start:i]))
		start, i = end, end
	}
	p.buffer = append(p.buffer[:0], p.buffer[start:]...)
	if last {
		parts = append(parts, p.buffer)
		p.buffer = nil
	}
	if !p.started {
		// Text before the first record separator is not a record.
		if (last && len(parts) <= 1) || (!last && len(parts) == 0) {
			return nil, nil
		}
		parts = parts[1:]
		p.started = true
	}
	var out [][]byte
	if p.kind == seqEvents {
		// The last part of the body is an event the stream ended in the middle of.
		if last && len(parts) > 0 {
			parts = parts[:len(parts)-1]
		}
		for _, line := range parts {
			event, err := p.line(strings.ToValidUTF8(string(line), "�"))
			if err != nil {
				return out, err
			}
			if event != nil {
				out = append(out, event)
			}
		}
		return out, nil
	}
	for _, part := range parts {
		p.count++
		if len(bytes.TrimSpace(part)) == 0 {
			continue
		}
		if !json.Valid(part) {
			what := "line"
			if p.kind == seqJSONSeq {
				what = "record"
			}
			return out, fmt.Errorf("tsurara: %s %d is not valid JSON", what, p.count)
		}
		out = append(out, part)
	}
	return out, nil
}

// line takes one line of an event stream and returns the event a blank line completes, as JSON.
func (p *parser) line(line string) ([]byte, error) {
	if line == "" {
		defer func() { p.event, p.data, p.id, p.retry = "", nil, nil, nil }()
		if len(p.data) == 0 {
			return nil, nil
		}
		name := p.event
		if name == "" {
			name = "message"
		}
		text := strings.Join(p.data, "\n")
		data := []byte(JSONText(text))
		if p.allJSON || containsString(p.jsonNames, name) {
			if !json.Valid([]byte(text)) {
				return nil, fmt.Errorf("tsurara: the data of the event %q is not valid JSON", name)
			}
			data = []byte(text)
		}
		var b bytes.Buffer
		b.WriteString(`{"event":`)
		b.WriteString(JSONText(name))
		b.WriteString(`,"data":`)
		b.Write(data)
		if p.id != nil {
			b.WriteString(`,"id":`)
			b.WriteString(JSONText(*p.id))
		}
		if p.retry != nil {
			b.WriteString(`,"retry":`)
			b.WriteString(strconv.FormatInt(*p.retry, 10))
		}
		b.WriteByte('}')
		return b.Bytes(), nil
	}
	if strings.HasPrefix(line, ":") {
		return nil, nil
	}
	field, value, _ := strings.Cut(line, ":")
	value = strings.TrimPrefix(value, " ")
	switch field {
	case "event":
		p.event = value
	case "data":
		p.data = append(p.data, value)
	case "id":
		if !strings.ContainsRune(value, 0) {
			p.id = &value
		}
	case "retry":
		if n, err := strconv.ParseInt(value, 10, 64); err == nil && value != "" && strings.Trim(value, "0123456789") == "" {
			p.retry = &n
		}
	}
	return nil, nil
}

func containsString(list []string, s string) bool {
	for _, x := range list {
		if x == s {
			return true
		}
	}
	return false
}
