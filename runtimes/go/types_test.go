package tsurara_test

import (
	"encoding/json"
	"testing"

	tsurara "github.com/tsurara-openapi/tsurara/runtimes/go"
)

// The helpers generated types call: these types are written as the Go backend writes them.

type Cat struct {
	PetType string `json:"petType"`
	Indoor  bool   `json:"indoor"`
}

type Dog struct {
	PetType string `json:"petType"`
}

type Pet struct {
	Cat     *Cat
	Dog     *Dog
	Unknown json.RawMessage
}

func (u Pet) MarshalJSON() ([]byte, error) {
	return tsurara.MarshalUnion(u.Cat, u.Dog, u.Unknown)
}

func (u *Pet) UnmarshalJSON(data []byte) error {
	*u = Pet{}
	return tsurara.UnmarshalTagged(data, "petType", []tsurara.TagCase{
		{Values: []string{"cat"}, Decode: tsurara.Into(&u.Cat)},
		{Values: []string{"dog", "doggo"}, Decode: tsurara.Into(&u.Dog)},
	}, tsurara.IntoRaw(&u.Unknown))
}

type ID struct {
	Integer *int64
	String  *string
	Cat     *Cat
	Null    bool
}

func (u ID) MarshalJSON() ([]byte, error) {
	return tsurara.MarshalUnion(u.Integer, u.String, u.Cat, tsurara.NullMember(u.Null))
}

func (u *ID) UnmarshalJSON(data []byte) error {
	*u = ID{}
	return tsurara.UnmarshalUntagged(data, tsurara.Into(&u.Integer), tsurara.Into(&u.String), tsurara.Into(&u.Cat), tsurara.IntoNull(&u.Null))
}

type Owner struct {
	Name                 string                   `json:"name"`
	Nick                 tsurara.Nullable[string] `json:"nick,omitzero"`
	AdditionalProperties map[string]int64         `json:"-"`
}

func (v Owner) MarshalJSON() ([]byte, error) {
	type plain Owner
	return tsurara.MarshalWithExtra(plain(v), v.AdditionalProperties)
}

func (v *Owner) UnmarshalJSON(data []byte) error {
	type plain Owner
	return tsurara.UnmarshalWithExtra(data, (*plain)(v), &v.AdditionalProperties)
}

func roundtrip[T any](t *testing.T, in string, check func(T)) {
	t.Helper()
	var v T
	if err := json.Unmarshal([]byte(in), &v); err != nil {
		t.Fatalf("%s: %v", in, err)
	}
	check(v)
	out, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	if !jsonEqual(string(out), in) {
		t.Fatalf("%s != %s", out, in)
	}
}

func jsonEqual(a, b string) bool {
	var x, y any
	_ = json.Unmarshal([]byte(a), &x)
	_ = json.Unmarshal([]byte(b), &y)
	return tsurara.JSONText(x) == tsurara.JSONText(y) || equalAny(x, y)
}

func equalAny(x, y any) bool {
	a, _ := json.Marshal(x)
	b, _ := json.Marshal(y)
	return string(a) == string(b)
}

func TestTaggedUnionsSelectByTheDiscriminator(t *testing.T) {
	roundtrip(t, `{"petType":"cat","indoor":true}`, func(p Pet) {
		if p.Cat == nil || !p.Cat.Indoor || p.Dog != nil {
			t.Fatalf("%+v", p)
		}
	})
	roundtrip(t, `{"petType":"doggo"}`, func(p Pet) {
		if p.Dog == nil || p.Dog.PetType != "doggo" {
			t.Fatalf("%+v", p)
		}
	})
	roundtrip(t, `{"petType":"bird","wings":2}`, func(p Pet) {
		if p.Cat != nil || p.Dog != nil || string(p.Unknown) != `{"petType":"bird","wings":2}` {
			t.Fatalf("%+v", p)
		}
	})
	var p Pet
	if err := json.Unmarshal([]byte(`[1]`), &p); err == nil {
		t.Fatal("an array is not a tagged union")
	}
	if _, err := json.Marshal(Pet{}); err == nil {
		t.Fatal("a union without a member set does not encode")
	}
}

func TestUntaggedUnionsTryMembersInOrder(t *testing.T) {
	roundtrip(t, `5`, func(u ID) {
		if u.Integer == nil || *u.Integer != 5 {
			t.Fatalf("%+v", u)
		}
	})
	roundtrip(t, `"x"`, func(u ID) {
		if u.String == nil || *u.String != "x" || u.Integer != nil {
			t.Fatalf("%+v", u)
		}
	})
	roundtrip(t, `{"petType":"cat","indoor":false}`, func(u ID) {
		if u.Cat == nil {
			t.Fatalf("%+v", u)
		}
	})
	roundtrip(t, `null`, func(u ID) {
		if !u.Null {
			t.Fatalf("%+v", u)
		}
	})
	var u ID
	for _, bad := range []string{`true`, `{"petType":"cat","wings":2}`, `1.5`} {
		if err := json.Unmarshal([]byte(bad), &u); err == nil {
			t.Fatalf("%s decoded as %+v", bad, u)
		}
	}
}

func TestAdditionalPropertiesAndNullable(t *testing.T) {
	roundtrip(t, `{"name":"a","x":1,"y":2}`, func(o Owner) {
		if o.Name != "a" || o.AdditionalProperties["x"] != 1 || len(o.AdditionalProperties) != 2 {
			t.Fatalf("%+v", o)
		}
		if _, set := o.Nick.Get(); set || o.Nick.IsNull() || !o.Nick.IsZero() {
			t.Fatalf("%+v", o.Nick)
		}
	})
	roundtrip(t, `{"name":"a","nick":null}`, func(o Owner) {
		if !o.Nick.IsNull() || o.AdditionalProperties != nil && len(o.AdditionalProperties) != 0 {
			t.Fatalf("%+v", o)
		}
	})
	roundtrip(t, `{"name":"a","nick":"n"}`, func(o Owner) {
		if v, ok := o.Nick.Get(); !ok || v != "n" {
			t.Fatalf("%+v", o.Nick)
		}
	})
	out, _ := json.Marshal(Owner{Name: "b", Nick: tsurara.Null[string](), AdditionalProperties: map[string]int64{"z": 1, "a": 2}})
	if string(out) != `{"name":"b","nick":null,"a":2,"z":1}` {
		t.Fatalf("%s", out)
	}
	out, _ = json.Marshal(Owner{Name: "c", Nick: tsurara.Value("v")})
	if string(out) != `{"name":"c","nick":"v"}` {
		t.Fatalf("%s", out)
	}
}
