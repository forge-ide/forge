package text

import "testing"

func TestTitle(t *testing.T) {
	if got := Title("forge"); got != "Forge" {
		t.Errorf("Title(%q) = %q", "forge", got)
	}
}

func TestSlugify(t *testing.T) {
	if got := Slugify("Hello, World!"); got != "hello-world" {
		t.Errorf("Slugify = %q, want %q", got, "hello-world")
	}
}
