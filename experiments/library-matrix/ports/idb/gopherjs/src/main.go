// Package main is idb 8.0.4 in Go, compiled to JavaScript by GopherJS: a port of the subset that
// libraries/idb/CONTRACT.md defines, and of what libraries/idb/reference.mjs does with it.
//
//   - idb.go: idb's layers (wrap, done, tx.store, the wrapped methods and cursor moves, openDB,
//     deleteDB, the database shortcuts and the async iterator);
//   - host.go: the IndexedDB, DOMException and Promise APIs idb drives;
//   - script.go: the corpus input, decoded into Go types;
//   - session.go: the script interpreter, its dump and its cleanup;
//   - this file: the export.
//
// The boundary: script.go decodes the input once. Keys, values and options stay *js.Object; the
// log is a JS array, and run returns a host Promise of {log, databases}. run is published on
// js.Module.exports, which the builder's facade binds and re-exports.
package main

import "github.com/gopherjs/gopherjs/js"

// jsError is what run rejects with: the host's error, or a TypeError for an input outside the
// contract.
func jsError(err error) *js.Object {
	if host, ok := err.(*js.Error); ok {
		return host.Object
	}
	return js.Global.Get("TypeError").New(err.Error())
}

func run(input *js.Object) *js.Object {
	return newPromise(func(resolve, reject func(*js.Object)) {
		steps, err := decode(input)
		if err != nil {
			reject(jsError(err))
			return
		}
		s := newSession()
		s.play(steps, func(output *js.Object, err error) {
			// As the reference's finally: cleanup always runs, and its own failure replaces the result.
			s.cleanup(func(cleanupErr error) {
				switch {
				case cleanupErr != nil:
					reject(jsError(cleanupErr))
				case err != nil:
					reject(jsError(err))
				default:
					resolve(output)
				}
			})
		})
	}).object
}

func main() { js.Module.Get("exports").Set("run", run) }
