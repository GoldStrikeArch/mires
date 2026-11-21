// idb 8.0.4 (build/index.js) for Go: IndexedDB requests become host Promises, a transaction gets a
// done Promise, cursors move by returning a Promise of the next position, and Iterate walks a
// store or an index.
//
// idb wraps each IDB object in a Proxy that forwards every property and wraps what comes back. Go
// has no Proxies, so each wrapped object is a typed handle over the host object, whose methods
// call the host method and wrap the result as idb's get trap would. idb's wrapped methods throw
// synchronously; here they return an error, and a request is issued when the method is called.

package main

import "github.com/gopherjs/gopherjs/js"

// wrap is idb's promisifyRequest: settled once, with the request's result on success or its
// error on error, after which both listeners are removed. A cursor's request fires once per
// move, so each move wraps the same request again.
func wrap(request Request) Promise {
	return newPromise(func(resolve, reject func(*js.Object)) {
		var success, failure *js.Object
		unlisten := func() {
			request.object.Call("removeEventListener", "success", success)
			request.object.Call("removeEventListener", "error", failure)
		}
		success = listener(func() {
			resolve(request.Result())
			unlisten()
		})
		failure = listener(func() {
			reject(request.Error())
			unlisten()
		})
		request.object.Call("addEventListener", "success", success)
		request.object.Call("addEventListener", "error", failure)
	})
}

// Method is a request method idb promisifies; the database shortcuts are their readMethods and
// writeMethods.
type Method string

const (
	Get        Method = "get"
	GetKey     Method = "getKey"
	GetAll     Method = "getAll"
	GetAllKeys Method = "getAllKeys"
	Count      Method = "count"
	Put        Method = "put"
	Add        Method = "add"
	Delete     Method = "delete"
	Clear      Method = "clear"
)

func (m Method) isWrite() bool { return m == Put || m == Add || m == Delete || m == Clear }

// Source is what an IDBObjectStore and an IDBIndex share: the request methods and cursors.
type Source struct{ object *js.Object }

// Raw is the unwrapped host call: it returns the raw request.
func (s Source) Raw(method Method, args ...any) (Request, error) {
	request, err := call(s.object, string(method), args...)
	return Request{request}, err
}

// Request is a wrapped request method, called with exactly the arguments given.
func (s Source) Request(method Method, args ...any) (Promise, error) {
	request, err := s.Raw(method, args...)
	if err != nil {
		return Promise{}, err
	}
	return wrap(request), nil
}

// OpenCursor resolves with the first cursor, or null when nothing matches.
func (s Source) OpenCursor(args ...any) (Promise, error) {
	request, err := call(s.object, "openCursor", args...)
	if err != nil {
		return Promise{}, err
	}
	return wrap(Request{request}), nil
}

func (s Source) OpenKeyCursor(args ...any) (Promise, error) {
	request, err := call(s.object, "openKeyCursor", args...)
	if err != nil {
		return Promise{}, err
	}
	return wrap(Request{request}), nil
}

// Iterate is store.iterate(query, direction).
func (s Source) Iterate(args ...any) *Iterate { return &Iterate{source: s, args: args} }

func (s Source) Name() string        { return s.object.Get("name").String() }
func (s Source) KeyPath() *js.Object { return s.object.Get("keyPath") }

type ObjectStore struct{ Source }

func (s ObjectStore) Index(name string) (Index, error) {
	index, err := call(s.object, "index", name)
	return Index{Source{index}}, err
}

func (s ObjectStore) CreateIndex(name string, keyPath, options *js.Object) (Index, error) {
	index, err := call(s.object, "createIndex", name, keyPath, options)
	return Index{Source{index}}, err
}

func (s ObjectStore) DeleteIndex(name string) error {
	_, err := call(s.object, "deleteIndex", name)
	return err
}

func (s ObjectStore) AutoIncrement() bool  { return s.object.Get("autoIncrement").Bool() }
func (s ObjectStore) IndexNames() []string { return names(s.object.Get("indexNames")) }

type Index struct{ Source }

func (i Index) Unique() bool     { return i.object.Get("unique").Bool() }
func (i Index) MultiEntry() bool { return i.object.Get("multiEntry").Bool() }

// Transaction is a wrapped IDBTransaction, with its done Promise.
type Transaction struct {
	object *js.Object
	Done   Promise
}

// wrapTransaction creates done when idb first wraps the transaction, so done also settles for a
// caller that only asks after the transaction finished.
func wrapTransaction(object *js.Object) *Transaction {
	return &Transaction{object, cacheDone(object)}
}

// cacheDone is idb's cacheDonePromiseForTransaction: settled by the first complete, error or
// abort. A request error bubbles to the transaction before the transaction records it, so done
// then rejects with a new AbortError.
func cacheDone(tx *js.Object) Promise {
	return newPromise(func(resolve, reject func(*js.Object)) {
		var complete, failure *js.Object
		unlisten := func() {
			tx.Call("removeEventListener", "complete", complete)
			tx.Call("removeEventListener", "error", failure)
			tx.Call("removeEventListener", "abort", failure)
		}
		complete = listener(func() {
			resolve(js.Undefined)
			unlisten()
		})
		failure = listener(func() {
			if err := tx.Get("error"); err != nil {
				reject(err)
			} else {
				reject(newDOMException("AbortError", "AbortError"))
			}
			unlisten()
		})
		tx.Call("addEventListener", "complete", complete)
		tx.Call("addEventListener", "error", failure)
		tx.Call("addEventListener", "abort", failure)
	})
}

func (tx *Transaction) ObjectStore(name string) (ObjectStore, error) {
	store, err := call(tx.object, "objectStore", name)
	return ObjectStore{Source{store}}, err
}

func (tx *Transaction) Abort() error {
	_, err := call(tx.object, "abort")
	return err
}

// Store is idb's tx.store trap, objectStoreNames[1] ? undefined : objectStore(objectStoreNames[0]):
// nil when the transaction has several stores. Store names are sorted and distinct, so a second
// name is never the empty string, and the test is the name count. On a finished transaction with
// one store, objectStore throws InvalidStateError.
func (tx *Transaction) Store() (*ObjectStore, error) {
	names := tx.object.Get("objectStoreNames")
	if names.Length() > 1 {
		return nil, nil
	}
	store, err := tx.ObjectStore(names.Index(0).String())
	if err != nil {
		return nil, err
	}
	return &store, nil
}

// Database is a wrapped IDBDatabase.
type Database struct{ object *js.Object }

func (db Database) Version() float64           { return db.object.Get("version").Float() }
func (db Database) ObjectStoreNames() []string { return names(db.object.Get("objectStoreNames")) }
func (db Database) Close()                     { db.object.Call("close") }

func (db Database) CreateObjectStore(name string, options *js.Object) (ObjectStore, error) {
	store, err := call(db.object, "createObjectStore", name, options)
	return ObjectStore{Source{store}}, err
}

func (db Database) DeleteObjectStore(name string) error {
	_, err := call(db.object, "deleteObjectStore", name)
	return err
}

// Transaction is db.transaction(stores, mode): stores is a name or an array of names, and an
// undefined mode is readonly.
func (db Database) Transaction(stores, mode any) (*Transaction, error) {
	tx, err := call(db.object, "transaction", stores, mode)
	if err != nil {
		return nil, err
	}
	return wrapTransaction(tx), nil
}

// Shortcut is a database shortcut (db.get(store, ...), db.getAllFromIndex(store, index, ...), ...),
// database-extras' getMethod: one transaction on tx.store (or its index) per call, then
// Promise.all([request, isWrite && tx.done]). So a write resolves only after done, the request's
// error comes first, and both promises are handled. The method is async: a synchronous failure
// rejects the call. index is nil for the store itself.
func (db Database) Shortcut(method Method, store string, index *string, args ...any) Promise {
	issued, err := db.shortcut(method, store, index, args)
	if err != nil {
		return rejected(err)
	}
	return issued
}

func (db Database) shortcut(method Method, store string, index *string, args []any) (Promise, error) {
	mode := "readonly"
	if method.isWrite() {
		mode = "readwrite"
	}
	tx, err := db.Transaction(store, mode)
	if err != nil {
		return Promise{}, err
	}
	target, err := tx.Store()
	if err != nil {
		return Promise{}, err
	}
	source := target.Source
	if index != nil {
		index, err := target.Index(*index)
		if err != nil {
			return Promise{}, err
		}
		source = index.Source
	}
	request, err := source.Request(method, args...)
	if err != nil {
		return Promise{}, err
	}
	var done any = false
	if method.isWrite() {
		done = tx.Done.object
	}
	return promiseAll(request.object, done).Map(func(results *js.Object) *js.Object { return results.Index(0) }), nil
}

// Cursor is a wrapped IDBCursor or IDBCursorWithValue.
type Cursor struct{ object *js.Object }

func (c Cursor) Key() *js.Object        { return c.object.Get("key") }
func (c Cursor) PrimaryKey() *js.Object { return c.object.Get("primaryKey") }
func (c Cursor) Value() *js.Object      { return c.object.Get("value") }

// moved is idb's cursor advance methods (wrapFunction): the host move, then the cursor's request
// wrapped again. The promise resolves with the same cursor, or with null past the end.
func (c Cursor) moved(method string, args ...any) (Promise, error) {
	if _, err := call(c.object, method, args...); err != nil {
		return Promise{}, err
	}
	return wrap(Request{c.object.Get("request")}), nil
}

func (c Cursor) Advance(count *js.Object) (Promise, error) { return c.moved("advance", count) }

// Continue is continue(key); an undefined key continues to the next record.
func (c Cursor) Continue(key *js.Object) (Promise, error) { return c.moved("continue", key) }

func (c Cursor) ContinuePrimaryKey(key, primaryKey *js.Object) (Promise, error) {
	return c.moved("continuePrimaryKey", key, primaryKey)
}

func (c Cursor) Update(value *js.Object) (Promise, error) {
	request, err := call(c.object, "update", value)
	if err != nil {
		return Promise{}, err
	}
	return wrap(Request{request}), nil
}

func (c Cursor) Delete() (Promise, error) {
	request, err := call(c.object, "delete")
	if err != nil {
		return Promise{}, err
	}
	return wrap(Request{request}), nil
}

// Iterate is idb's iterate, the async iterator of a store or an index. The first Next opens
// openCursor(...args). Each later one awaits the move the loop body made on the yielded cursor,
// or calls continue() when the body made none.
type Iterate struct {
	source   Source
	args     []any
	state    iterState
	cursor   Cursor
	advanced *Promise // idb's advanceResults entry
}

type iterState int

const (
	unopened iterState = iota
	positioned
	finished
)

// Next is the iterator's next(): a Promise of the next position's cursor, or of null at the end.
// The cursor it yields is Cursor.
func (it *Iterate) Next() Promise {
	state := it.state
	// While the move is pending the generator is suspended; when it fails, the generator is done.
	it.state = finished
	var moved Promise
	var err error
	switch {
	case state == finished:
		return resolved(nil)
	case state == unopened:
		moved, err = it.source.OpenCursor(it.args...)
	case it.advanced != nil:
		moved = *it.advanced
	default:
		moved, err = it.cursor.Continue(js.Undefined)
	}
	if err != nil {
		return rejected(err)
	}
	return moved.Map(func(cursor *js.Object) *js.Object {
		it.advanced = nil
		if cursor != nil {
			it.state = positioned
			it.cursor = Cursor{cursor}
		}
		return cursor
	})
}

// Cursor is the cursor the iteration yields, idb's proxied cursor: the same one at every
// position. Its moves are intercepted.
func (it *Iterate) Cursor() IterCursor { return IterCursor{it.cursor, it} }

// IterCursor forwards everything to the cursor except its moves: they call the host move and
// record its promise for the iterator's next step, as idb's cursorIteratorTraps do.
type IterCursor struct {
	Cursor
	iterate *Iterate
}

func (c IterCursor) record(moved Promise, err error) error {
	if err == nil {
		c.iterate.advanced = &moved
	}
	return err
}

func (c IterCursor) Advance(count *js.Object) error { return c.record(c.Cursor.Advance(count)) }
func (c IterCursor) Continue(key *js.Object) error  { return c.record(c.Cursor.Continue(key)) }

func (c IterCursor) ContinuePrimaryKey(key, primaryKey *js.Object) error {
	return c.record(c.Cursor.ContinuePrimaryKey(key, primaryKey))
}

// OpenCallbacks are openDB's callbacks; a nil one is absent. Upgrade gets (db, oldVersion,
// newVersion, transaction); Blocked and Blocking get (currentVersion, blockedVersion, event), and
// blockedVersion is nil for a deletion.
type OpenCallbacks struct {
	Upgrade  func(db Database, oldVersion float64, newVersion *float64, tx *Transaction)
	Blocked  func(currentVersion float64, blockedVersion *float64, event *js.Object)
	Blocking func(currentVersion float64, blockedVersion *float64, event *js.Object)
}

// OpenDB is idb's openDB(name, version, { upgrade, blocked, blocking }): a Promise of the
// IDBDatabase. An undefined version opens the current version, or version 1 for a new database;
// an invalid one fails now. Blocking is attached to the connection once it is open.
func OpenDB(name string, version *js.Object, callbacks OpenCallbacks) (Promise, error) {
	request, err := openDatabase(name, version)
	if err != nil {
		return Promise{}, err
	}
	openPromise := wrap(request)
	if upgrade := callbacks.Upgrade; upgrade != nil {
		onVersionChange(request.object, "upgradeneeded", func(oldVersion float64, newVersion *float64, _ *js.Object) {
			upgrade(Database{request.Result()}, oldVersion, newVersion, wrapTransaction(request.object.Get("transaction")))
		})
	}
	if blocked := callbacks.Blocked; blocked != nil {
		onVersionChange(request.object, "blocked", blocked)
	}
	openPromise.Map(func(db *js.Object) *js.Object {
		if blocking := callbacks.Blocking; blocking != nil {
			onVersionChange(db, "versionchange", blocking)
		}
		return nil
	}).IgnoreRejection()
	return openPromise, nil
}

// DeleteDB is idb's deleteDB(name, { blocked }): blocked(currentVersion, event), or nil; resolves
// undefined.
func DeleteDB(name string, blocked func(currentVersion float64, event *js.Object)) (Promise, error) {
	request, err := deleteDatabase(name)
	if err != nil {
		return Promise{}, err
	}
	if blocked != nil {
		onVersionChange(request.object, "blocked", func(oldVersion float64, _ *float64, event *js.Object) {
			blocked(oldVersion, event)
		})
	}
	return wrap(request).Map(func(*js.Object) *js.Object { return js.Undefined }), nil
}
