// The session plays a case's script through the port's idb, as libraries/idb/reference.mjs drives
// npm idb, then dumps the databases it left, and closes and deletes every database it touched,
// also after a failure.
//
// Every continuation runs from a host Promise callback, as a microtask: a step's work is
// continuation-passing, each step calling done with nil, a logged{} failure, or an input outside
// the contract (a contractError, never logged).

package main

import "github.com/gopherjs/gopherjs/js"

// logged is a failure that the step where it happened has already logged.
type logged struct{}

func (logged) Error() string { return "a logged failure" }

type session struct {
	prefix      string
	log         *js.Object
	connections map[string]Database
	opened      []Database
	attempted   []string
	live        []string
}

func newSession() *session {
	uuid := js.Global.Get("crypto").Call("randomUUID").String()
	return &session{prefix: "mires-matrix-idb-" + uuid + "-", log: newArray(), connections: map[string]Database{}}
}

func remember(names []string, name string) []string {
	for _, known := range names {
		if known == name {
			return names
		}
	}
	return append(names, name)
}

func (s *session) connection(conn string) (Database, error) {
	db, ok := s.connections[conn]
	if !ok {
		return Database{}, contractError("no connection named " + conn)
	}
	return db, nil
}

// --- The log ---

func (s *session) push(entry ...any) { s.log.Call("push", newArray(entry...)) }

// ok logs [label, "ok"] for an undefined result, [label, "ok", value] otherwise.
func (s *session) ok(label string, value *js.Object) {
	if value == js.Undefined {
		s.push(label, "ok")
	} else {
		s.push(label, "ok", value)
	}
}

func (s *session) logError(label string, err error) {
	s.push(label, "error", errorName(err.(*js.Error)))
}

// fail logs a host error where it happened and ends the step; an input outside the contract is
// passed on, unlogged.
func (s *session) fail(label string, err error, done func(error)) {
	if _, host := err.(*js.Error); host {
		s.logError(label, err)
		err = logged{}
	}
	done(err)
}

// await follows an issued call: its synchronous failure or its rejection ends the step, logged
// with label; its value goes to next.
func (s *session) await(label string, promise Promise, err error, done func(error), next func(*js.Object)) {
	if err != nil {
		s.fail(label, err, done)
		return
	}
	promise.Then(func(value *js.Object, err *js.Error) {
		if err != nil {
			s.fail(label, err, done)
			return
		}
		next(value)
	})
}

// attempt awaits an issued call, then logs it.
func (s *session) attempt(label string, promise Promise, err error, done func(error)) {
	s.await(label, promise, err, done, func(value *js.Object) {
		s.ok(label, value)
		done(nil)
	})
}

func version(v *float64) any {
	if v == nil {
		return nil
	}
	return *v
}

func stringArray(names []string) *js.Object {
	array := newArray()
	for _, name := range names {
		array.Call("push", name)
	}
	return array
}

// --- Sources and arguments ---

// source is tx.objectStore(store), or tx.store, then .index(index).
func source(tx *Transaction, target Target) (Source, error) {
	var store ObjectStore
	if target.Store != nil {
		var err error
		if store, err = tx.ObjectStore(*target.Store); err != nil {
			return Source{}, err
		}
	} else {
		only, err := tx.Store()
		if err != nil {
			return Source{}, err
		}
		if only == nil {
			return Source{}, contractError("store: null in a transaction with several stores")
		}
		store = *only
	}
	if target.Index == nil {
		return store.Source, nil
	}
	index, err := store.Index(*target.Index)
	return index.Source, err
}

// hostArgs builds the key ranges among a call's arguments: a DataError here is the step's failure.
func hostArgs(args []any) ([]any, error) {
	built := make([]any, len(args))
	for i, arg := range args {
		if r, ok := arg.(keyRange); ok {
			value, err := r.build()
			if err != nil {
				return nil, err
			}
			built[i] = value
		} else {
			built[i] = arg
		}
	}
	return built, nil
}

// issue is source[method](...args): the source is resolved first, then the arguments are built.
func issue(tx *Transaction, r RequestStep) (Promise, error) {
	source, err := source(tx, r.Target)
	if err != nil {
		return Promise{}, err
	}
	args, err := hostArgs(r.Args)
	if err != nil {
		return Promise{}, err
	}
	return source.Request(r.Method, args...)
}

// --- Steps ---

// performAll runs steps in order without growing the JS call stack on synchronous steps (close,
// store, abort, or caught synchronous host errors): in Firefox SpiderMonkey caps Error().stack at
// 128 frames, which breaks GopherJS's $recover() check in catch() if synchronous steps recurse.
func (s *session) performAll(steps []Step, tx *Transaction, done func(error)) {
	index := 0
	var advance func()
	advance = func() {
		for index < len(steps) {
			current := steps[index]
			index++
			sync := true
			settled := false
			var syncErr error
			s.perform(current, tx, func(err error) {
				if sync {
					settled = true
					syncErr = err
					return
				}
				if err != nil {
					done(err)
					return
				}
				advance()
			})
			sync = false
			if !settled {
				return
			}
			if syncErr != nil {
				done(syncErr)
				return
			}
		}
		done(nil)
	}
	advance()
}

func (s *session) perform(step Step, tx *Transaction, done func(error)) {
	s.execute(step.Op, tx, func(err error) {
		if err == (logged{}) && step.Catch {
			err = nil
		}
		done(err)
	})
}

func (s *session) execute(op Op, tx *Transaction, done func(error)) {
	switch op := op.(type) {
	case OpenDBStep:
		s.openDB(op, done)
		return
	case CloseStep:
		db, err := s.connection(op.Conn)
		if err != nil {
			done(err)
			return
		}
		db.Close()
		s.ok("close", js.Undefined)
		done(nil)
		return
	case DeleteDBStep:
		s.deleteDB(op, done)
		return
	case TransactionStep:
		db, err := s.connection(op.Conn)
		if err != nil {
			done(err)
			return
		}
		inner, err := db.Transaction(op.Stores, op.Mode)
		if err != nil {
			s.fail("tx", err, done)
			return
		}
		s.performAll(op.Steps, inner, done)
		return
	case ShortcutStep:
		db, err := s.connection(op.Conn)
		if err != nil {
			done(err)
			return
		}
		args, err := hostArgs(op.Args)
		if err != nil {
			s.fail(op.Label, err, done)
			return
		}
		s.attempt(op.Label, db.Shortcut(op.Method, op.Store, op.Index, args...), nil, done)
		return
	}
	if tx == nil {
		done(contractError("a transaction step outside a transaction"))
		return
	}
	switch op := op.(type) {
	case RequestStep:
		promise, err := issue(tx, op)
		s.attempt(string(op.Method), promise, err, done)
	case BatchStep:
		s.batch(tx, op, done)
	case WalkStep:
		if op.Kind == IterateWalk {
			s.iterate(tx, op, done)
		} else {
			s.walk(tx, op, done)
		}
	case AbortStep:
		if err := tx.Abort(); err != nil {
			s.fail("abort", err, done)
			return
		}
		s.ok("abort", js.Undefined)
		done(nil)
	case DoneStep:
		s.attempt("done", tx.Done, nil, done)
	case StoreStep:
		store, err := tx.Store()
		switch {
		case err != nil:
			s.fail("store", err, done)
			return
		case store == nil:
			s.push("store", "ok")
		default:
			s.push("store", "ok", store.Name())
		}
		done(nil)
	case WrapStep:
		// A wrapped store or index unwraps to its host object, whose method returns the raw request.
		wrapped := func() (Promise, error) {
			source, err := source(tx, op.Target)
			if err != nil {
				return Promise{}, err
			}
			args, err := hostArgs(op.Args)
			if err != nil {
				return Promise{}, err
			}
			request, err := source.Raw(op.Method, args...)
			if err != nil {
				return Promise{}, err
			}
			return wrap(request), nil
		}
		promise, err := wrapped()
		s.attempt("wrap", promise, err, done)
	}
}

func (s *session) openDB(op OpenDBStep, done func(error)) {
	s.attempted = remember(s.attempted, op.DB)
	var closing *Database
	if op.Blocked != nil && op.Blocked.Close != nil {
		db, err := s.connection(*op.Blocked.Close)
		if err != nil {
			done(err)
			return
		}
		closing = &db
	}
	var callbacks OpenCallbacks
	if op.HasUpgrade {
		callbacks.Upgrade = func(db Database, oldVersion float64, newVersion *float64, tx *Transaction) {
			s.push("upgrade", oldVersion, version(newVersion))
			for _, step := range op.Upgrade {
				s.upgrade(step, db, tx)
			}
		}
	}
	if op.Blocked != nil {
		callbacks.Blocked = func(currentVersion float64, blockedVersion *float64, _ *js.Object) {
			s.push("blocked", currentVersion, version(blockedVersion))
			if closing != nil {
				closing.Close()
			}
		}
	}
	if blocking := op.Blocking; blocking != nil {
		callbacks.Blocking = func(currentVersion float64, blockedVersion *float64, event *js.Object) {
			s.push("blocking", op.Conn, currentVersion, version(blockedVersion))
			if blocking.Close {
				Database{event.Get("target")}.Close()
			}
		}
	}
	opened, err := OpenDB(s.prefix+op.DB, op.Version, callbacks)
	s.await("openDB", opened, err, done, func(value *js.Object) {
		db := Database{value}
		s.push("openDB", "ok", newArray(db.Version(), stringArray(db.ObjectStoreNames())))
		s.connections[op.Conn] = db
		s.opened = append(s.opened, db)
		s.live = remember(s.live, op.DB)
		done(nil)
	})
}

func (s *session) deleteDB(op DeleteDBStep, done func(error)) {
	s.attempted = remember(s.attempted, op.DB)
	var blocked func(float64, *js.Object)
	if op.Blocked != nil {
		var closing *Database
		if op.Blocked.Close != nil {
			db, err := s.connection(*op.Blocked.Close)
			if err != nil {
				done(err)
				return
			}
			closing = &db
		}
		blocked = func(currentVersion float64, _ *js.Object) {
			s.push("blocked", currentVersion)
			if closing != nil {
				closing.Close()
			}
		}
	}
	deleted, err := DeleteDB(s.prefix+op.DB, blocked)
	s.await("deleteDB", deleted, err, done, func(*js.Object) {
		s.ok("deleteDB", js.Undefined)
		live := s.live[:0]
		for _, name := range s.live {
			if name != op.DB {
				live = append(live, name)
			}
		}
		s.live = live
		done(nil)
	})
}

// upgrade runs inside the upgrade callback: synchronous calls, each logged; a failure does not
// stop it.
func (s *session) upgrade(step UpgradeStep, db Database, tx *Transaction) {
	switch step := step.(type) {
	case CreateStore:
		if store, err := db.CreateObjectStore(step.Name, step.Options); err != nil {
			s.logError("createStore", err)
		} else {
			s.push("createStore", "ok", store.Name())
		}
	case DeleteStore:
		if err := db.DeleteObjectStore(step.Name); err != nil {
			s.logError("deleteStore", err)
		} else {
			s.push("deleteStore", "ok")
		}
	case CreateIndex:
		store, err := tx.ObjectStore(step.Store)
		var index Index
		if err == nil {
			index, err = store.CreateIndex(step.Name, step.KeyPath, step.Options)
		}
		if err != nil {
			s.logError("createIndex", err)
		} else {
			s.push("createIndex", "ok", index.Name())
		}
	case DeleteIndex:
		store, err := tx.ObjectStore(step.Store)
		if err == nil {
			err = store.DeleteIndex(step.Name)
		}
		if err != nil {
			s.logError("deleteIndex", err)
		} else {
			s.push("deleteIndex", "ok")
		}
	case UpgradeAbort:
		if err := tx.Abort(); err != nil {
			s.logError("abort", err)
		} else {
			s.push("abort", "ok")
		}
	case UpgradeWrite:
		// Issued, not awaited: a failed write aborts the upgrade, which rejects the open.
		store, err := tx.ObjectStore(step.Store)
		var written Promise
		if err == nil {
			written, err = store.Request(step.Method, step.Args...)
		}
		if err != nil {
			s.logError(string(step.Method), err)
			return
		}
		written.IgnoreRejection()
	}
}

// batch issues every request before awaiting any, then logs each outcome in issue order, as
// Promise.allSettled reports them. It never fails.
func (s *session) batch(tx *Transaction, op BatchStep, done func(error)) {
	var pending []Promise
	var labels []string
	for _, r := range op.Requests {
		promise, err := issue(tx, r)
		if err != nil {
			if _, host := err.(*js.Error); !host {
				done(err)
				return
			}
			// The reference issues each request in an async function: a synchronous failure is a
			// rejection.
			promise = rejected(err)
		}
		pending = append(pending, promise)
		labels = append(labels, string(r.Method))
	}
	if op.Abort {
		if err := tx.Abort(); err != nil {
			s.logError("abort", err)
		} else {
			s.ok("abort", js.Undefined)
		}
	}
	if op.Done {
		pending = append(pending, tx.Done)
		labels = append(labels, "done")
	}
	allSettled(pending).Then(func(outcomes *js.Object, _ *js.Error) {
		for i, outcome := range elements(outcomes) {
			if outcome.Get("status").String() == "rejected" {
				s.logError(labels[i], &js.Error{Object: outcome.Get("reason")})
			} else {
				s.ok(labels[i], outcome.Get("value"))
			}
		}
		done(nil)
	})
}

// --- Cursor walks ---

func (s *session) position(kind WalkKind, cursor Cursor) {
	if kind == KeyCursorWalk {
		s.push("cursor", cursor.Key(), cursor.PrimaryKey())
	} else {
		s.push("cursor", cursor.Key(), cursor.PrimaryKey(), cursor.Value())
	}
}

// write is cursor.update(value) or cursor.delete(), awaited and logged.
func (s *session) write(cursor Cursor, action Action, done func(error), next func()) {
	var promise Promise
	var err error
	if action.Name == ActionUpdate {
		promise, err = cursor.Update(action.arg(0))
	} else {
		promise, err = cursor.Delete()
	}
	s.await(string(action.Name), promise, err, done, func(value *js.Object) {
		s.ok(string(action.Name), value)
		next()
	})
}

// A move is the promise a walk awaits after its position's actions, and the move's name.
type move struct {
	label string
	next  Promise
}

func (s *session) walk(tx *Transaction, w WalkStep, done func(error)) {
	opened := func() (Promise, error) {
		source, err := source(tx, w.Target)
		if err != nil {
			return Promise{}, err
		}
		args, err := hostArgs(w.Args)
		if err != nil {
			return Promise{}, err
		}
		if w.Kind == KeyCursorWalk {
			return source.OpenKeyCursor(args...)
		}
		return source.OpenCursor(args...)
	}
	promise, err := opened()
	s.await(string(w.Kind), promise, err, done, func(cursor *js.Object) { s.walkFrom(w, cursor, 0, done) })
}

// walkFrom is a walk with the promise-returning moves: each position's move is awaited after its
// actions.
func (s *session) walkFrom(w WalkStep, value *js.Object, at int, done func(error)) {
	if value == nil {
		s.push("end")
		done(nil)
		return
	}
	cursor := Cursor{value}
	s.position(w.Kind, cursor)
	s.walkActions(cursor, w.actions(at), nil, done, func(moved *move) {
		if moved == nil {
			done(nil)
			return
		}
		s.await(moved.label, moved.next, nil, done, func(next *js.Object) { s.walkFrom(w, next, at+1, done) })
	})
}

// walkActions runs a position's actions one at a time, then calls next with the move to await,
// or nil when the walk stops here: no move, or a break. A failing action ends the walk.
func (s *session) walkActions(cursor Cursor, actions []Action, moved *move, done func(error), next func(*move)) {
	for i, action := range actions {
		var promise Promise
		var err error
		switch action.Name {
		case ActionBreak:
			next(nil)
			return
		case ActionUpdate, ActionDelete:
			s.write(cursor, action, done, func() { s.walkActions(cursor, actions[i+1:], moved, done, next) })
			return
		case ActionAdvance:
			promise, err = cursor.Advance(action.arg(0))
		case ActionContinuePrimaryKey:
			promise, err = cursor.ContinuePrimaryKey(action.arg(0), action.arg(1))
		default:
			promise, err = cursor.Continue(action.arg(0))
		}
		if err != nil {
			s.fail(string(action.Name), err, done)
			return
		}
		moved = &move{string(action.Name), promise}
	}
	next(moved)
}

func (s *session) iterate(tx *Transaction, w WalkStep, done func(error)) {
	source, err := source(tx, w.Target)
	if err != nil {
		s.fail("iterate", err, done)
		return
	}
	args, err := hostArgs(w.Args)
	if err != nil {
		s.fail("iterate", err, done)
		return
	}
	s.iterateFrom(w, source.Iterate(args...), 0, done)
}

// iterateFrom is the async iterator: a position without a move continues.
func (s *session) iterateFrom(w WalkStep, it *Iterate, at int, done func(error)) {
	s.await("iterate", it.Next(), nil, done, func(value *js.Object) {
		if value == nil {
			s.push("end")
			done(nil)
			return
		}
		cursor := it.Cursor()
		s.position(w.Kind, cursor.Cursor)
		s.iterateActions(cursor, w.actions(at), done, func(continues bool) {
			if continues {
				s.iterateFrom(w, it, at+1, done)
			} else {
				done(nil)
			}
		})
	})
}

// iterateActions runs a position's actions in the iterator: a move only records the next step.
// next gets false on a break.
func (s *session) iterateActions(cursor IterCursor, actions []Action, done func(error), next func(continues bool)) {
	for i, action := range actions {
		var err error
		switch action.Name {
		case ActionBreak:
			next(false)
			return
		case ActionUpdate, ActionDelete:
			s.write(cursor.Cursor, action, done, func() { s.iterateActions(cursor, actions[i+1:], done, next) })
			return
		case ActionAdvance:
			err = cursor.Advance(action.arg(0))
		case ActionContinuePrimaryKey:
			err = cursor.ContinuePrimaryKey(action.arg(0), action.arg(1))
		default:
			err = cursor.Continue(action.arg(0))
		}
		if err != nil {
			s.fail(string(action.Name), err, done)
			return
		}
	}
	next(true)
}

// --- Dump and cleanup ---

// dump is [db, version, stores], read through a connection of its own, closed also after a
// failure.
func (s *session) dump(name string, done func(*js.Object, error)) {
	opened, err := OpenDB(s.prefix+name, js.Undefined, OpenCallbacks{})
	if err != nil {
		done(nil, err)
		return
	}
	opened.Then(func(value *js.Object, err *js.Error) {
		if err != nil {
			done(nil, err)
			return
		}
		db := Database{value}
		dumpStores(db, func(stores *js.Object, err error) {
			db.Close()
			if err != nil {
				done(nil, err)
				return
			}
			done(newArray(name, db.Version(), stores), nil)
		})
	})
}

// dumpStores is [storeName, keyPath, autoIncrement, indexes, records] for each store, in one
// readonly transaction.
func dumpStores(db Database, done func(*js.Object, error)) {
	stores := newArray()
	names := db.ObjectStoreNames()
	if len(names) == 0 {
		done(stores, nil)
		return
	}
	tx, err := db.Transaction(stringArray(names), js.Undefined)
	if err != nil {
		done(nil, err)
		return
	}
	var next func(i int)
	next = func(i int) {
		if i == len(names) {
			tx.Done.Then(func(_ *js.Object, err *js.Error) {
				if err != nil {
					done(nil, err)
					return
				}
				done(stores, nil)
			})
			return
		}
		dumpStore(tx, names[i], func(store *js.Object, err error) {
			if err != nil {
				done(nil, err)
				return
			}
			stores.Call("push", store)
			next(i + 1)
		})
	}
	next(0)
}

func dumpStore(tx *Transaction, name string, done func(*js.Object, error)) {
	store, err := tx.ObjectStore(name)
	if err != nil {
		done(nil, err)
		return
	}
	indexes := newArray()
	for _, indexName := range store.IndexNames() {
		index, err := store.Index(indexName)
		if err != nil {
			done(nil, err)
			return
		}
		indexes.Call("push", newArray(indexName, index.KeyPath(), index.Unique(), index.MultiEntry()))
	}
	allKeys, err := store.Request(GetAllKeys)
	if err != nil {
		done(nil, err)
		return
	}
	allKeys.Then(func(keys *js.Object, err *js.Error) {
		if err != nil {
			done(nil, err)
			return
		}
		allValues, err2 := store.Request(GetAll)
		if err2 != nil {
			done(nil, err2)
			return
		}
		allValues.Then(func(values *js.Object, err *js.Error) {
			if err != nil {
				done(nil, err)
				return
			}
			records := newArray()
			for i, key := range elements(keys) {
				records.Call("push", newArray(key, values.Index(i)))
			}
			done(newArray(name, store.KeyPath(), store.AutoIncrement(), indexes, records), nil)
		})
	})
}

func (s *session) play(steps []Step, done func(*js.Object, error)) {
	s.performAll(steps, nil, func(err error) {
		// A logged failure ends the script there; anything else rejects the call.
		if err != nil && err != (logged{}) {
			done(nil, err)
			return
		}
		databases := newArray()
		var next func(i int)
		next = func(i int) {
			if i == len(s.live) {
				output := js.Global.Get("Object").New()
				output.Set("log", s.log)
				output.Set("databases", databases)
				done(output, nil)
				return
			}
			s.dump(s.live[i], func(dump *js.Object, err error) {
				if err != nil {
					done(nil, err)
					return
				}
				databases.Call("push", dump)
				next(i + 1)
			})
		}
		next(0)
	})
}

func (s *session) cleanup(done func(error)) {
	for _, db := range s.opened {
		db.Close()
	}
	var next func(i int)
	next = func(i int) {
		if i == len(s.attempted) {
			done(nil)
			return
		}
		deleted, err := DeleteDB(s.prefix+s.attempted[i], nil)
		if err != nil {
			done(err)
			return
		}
		deleted.Then(func(_ *js.Object, err *js.Error) {
			if err != nil {
				done(err)
				return
			}
			next(i + 1)
		})
	}
	next(0)
}
