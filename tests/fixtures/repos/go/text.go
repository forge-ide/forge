// Package text has small text helpers.
package text

import "strings"

// Title returns name with its first letter capitalized.
func Title(name string) string {
	if name == "" {
		return ""
	}
	return strings.ToUpper(name[:1]) + name[1:]
}
