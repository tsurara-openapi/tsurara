package tsuraraplugin

import (
	"fmt"
	"go/format"
	"sort"
	"strings"
)

// File writes a Go file with its imports; Render formats it as gofmt would.
type File struct {
	Path    string
	Package string
	imports map[string]string
	lines   []string
}

// NewFile is a Go file at path (relative to the output root) in package pkg.
func NewFile(path, pkg string) *File {
	return &File{Path: path, Package: pkg, imports: map[string]string{}}
}

// Import adds an import, with an alias when alias is not empty.
func (f *File) Import(path, alias string) {
	f.imports[path] = alias
}

// Line appends a line of code.
func (f *File) Line(code string) *File {
	f.lines = append(f.lines, code)
	return f
}

// Render returns the file, starting with marker, formatted with go/format.
func (f *File) Render(marker string) (string, error) {
	var b strings.Builder
	fmt.Fprintf(&b, "%s\n\npackage %s\n", marker, f.Package)
	if len(f.imports) > 0 {
		paths := make([]string, 0, len(f.imports))
		for p := range f.imports {
			paths = append(paths, p)
		}
		sort.Strings(paths)
		b.WriteString("\nimport (\n")
		for _, p := range paths {
			if a := f.imports[p]; a != "" {
				fmt.Fprintf(&b, "\t%s %q\n", a, p)
			} else {
				fmt.Fprintf(&b, "\t%q\n", p)
			}
		}
		b.WriteString(")\n")
	}
	if len(f.lines) > 0 {
		b.WriteString("\n" + strings.Join(f.lines, "\n") + "\n")
	}
	out, err := format.Source([]byte(b.String()))
	return string(out), err
}
