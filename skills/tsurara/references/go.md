# Go

Each operation is a function in the generated package: `ctx`, the client, required parameters, the body, then `*{Op}Options` (or `nil`). It returns `(*tsurara.Success[T], error)`; failures are `*tsurara.Failure[E]`.

```go
package api

import (
	"context"
	"errors"
	"fmt"

	tsurara "github.com/tsurara-openapi/tsurara/runtimes/go"
)

// PetName returns the name of a pet, or "" when it cannot be read.
func PetName(ctx context.Context, c *tsurara.Client, petID string) string {
	res, err := ShowPetByID(ctx, c, petID, nil)
	var failure *tsurara.Failure[Error]
	switch {
	case err == nil:
		return res.Data.Name
	case errors.As(err, &failure) && failure.Kind == tsurara.KindHTTP && failure.Documented:
		fmt.Println(failure.Status, failure.Body.Message)
	default:
		fmt.Println(err)
	}
	return ""
}

// NewPetClient creates the client for the API.
func NewPetClient() *tsurara.Client {
	return tsurara.NewClient("https://petstore.example.com/v1")
}
```

- Optional fields are pointers; types without a Go equivalent (unions) are wrapper structs with methods.
- `tsurara.NewClient` takes options: `WithHeader`, `WithTimeout`, `WithRetry`, `WithAuth`.
- Tests: with `go.mocks: true`, `mocks_gen.go` (build tag `tsurara_mocks`, so run `go test -tags tsurara_mocks`) has `Mock{Type}()`, a constructor per documented status and `{Op}Route(respond)` for `tsuraratest` (`docs/languages/go.md`, "Mocks").
