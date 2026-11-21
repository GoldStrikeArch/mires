// The corpus input (CONTRACT "Input") as Go types. decode reads the input once and rejects
// anything outside the contract's schema, before any database is opened. Names become Go (UTF-8)
// strings; keys, values, versions, counts, directions, modes and options stay *js.Object and
// reach IndexedDB as they are, an absent one js.Undefined.

package main

import (
	"strings"

	"github.com/gopherjs/gopherjs/js"
)

// Step is one step of a script. A failure of a step with Catch, or of a step inside it, stops
// there and the script goes on after it.
type Step struct {
	Op    Op
	Catch bool
}

// Op is one of the step types below.
type Op interface{ isOp() }

type OpenDBStep struct {
	DB, Conn string
	Version  *js.Object // undefined when absent
	// HasUpgrade is an upgrade callback, which runs Upgrade (perhaps no steps at all).
	HasUpgrade bool
	Upgrade    []UpgradeStep
	Blocking   *Blocking
	Blocked    *Blocked
}

// Blocking is openDB's blocking callback: it logs, then with Close closes its own connection.
type Blocking struct{ Close bool }

// Blocked is a blocked callback: it logs, then closes the connection named Close, if any.
type Blocked struct{ Close *string }

type CloseStep struct{ Conn string }

type DeleteDBStep struct {
	DB      string
	Blocked *Blocked
}

type TransactionStep struct {
	Conn         string
	Stores, Mode *js.Object
	Steps        []Step
}

// ShortcutStep is a database shortcut; Label is its name, such as getAllFromIndex.
type ShortcutStep struct {
	Conn, Label string
	Method      Method
	Store       string
	Index       *string
	Args        []any
}

// RequestStep calls Method on its target. A query method's first argument is a key or a keyRange.
type RequestStep struct {
	Method Method
	Target Target
	Args   []any
}

// Target is tx.objectStore(Store), or tx.store when Store is nil, then .index(Index) unless Index
// is nil.
type Target struct{ Store, Index *string }

type BatchStep struct {
	Requests    []RequestStep
	Abort, Done bool
}

// WalkKind is the step's op, which also labels the failure of opening the cursor.
type WalkKind string

const (
	CursorWalk    WalkKind = "openCursor"
	KeyCursorWalk WalkKind = "openKeyCursor"
	IterateWalk   WalkKind = "iterate"
)

type WalkStep struct {
	Kind      WalkKind
	Target    Target
	Args      []any
	Positions [][]Action
	Rest      []Action
}

// actions are the actions at the position.
func (w WalkStep) actions(position int) []Action {
	if position < len(w.Positions) {
		return w.Positions[position]
	}
	return w.Rest
}

// ActionName is a cursor action, named as the script names it.
type ActionName string

const (
	ActionContinue           ActionName = "continue"
	ActionAdvance            ActionName = "advance"
	ActionContinuePrimaryKey ActionName = "continuePrimaryKey"
	ActionUpdate             ActionName = "update"
	ActionDelete             ActionName = "delete"
	ActionBreak              ActionName = "break"
)

// Action is a cursor action and its arguments: continue takes none or a key, advance a count,
// continuePrimaryKey a key and a primary key, update a value.
type Action struct {
	Name ActionName
	Args []*js.Object
}

// arg is the i-th argument, js.Undefined when the action has fewer.
func (a Action) arg(i int) *js.Object {
	if i < len(a.Args) {
		return a.Args[i]
	}
	return js.Undefined
}

type AbortStep struct{}
type DoneStep struct{}
type StoreStep struct{}

// WrapStep is wrap(unwrap(source)[method](...args)).
type WrapStep struct{ RequestStep }

func (OpenDBStep) isOp()      {}
func (CloseStep) isOp()       {}
func (DeleteDBStep) isOp()    {}
func (TransactionStep) isOp() {}
func (ShortcutStep) isOp()    {}
func (RequestStep) isOp()     {}
func (BatchStep) isOp()       {}
func (WalkStep) isOp()        {}
func (AbortStep) isOp()       {}
func (DoneStep) isOp()        {}
func (StoreStep) isOp()       {}
func (WrapStep) isOp()        {}

// UpgradeStep is one of the upgrade step types below; they run inside the upgrade callback.
type UpgradeStep interface{ isUpgrade() }

type CreateStore struct {
	Name    string
	Options *js.Object
}

type DeleteStore struct{ Name string }

type CreateIndex struct {
	Store, Name      string
	KeyPath, Options *js.Object
}

type DeleteIndex struct{ Store, Name string }

// UpgradeWrite is put or add in the upgrade transaction: issued, not awaited.
type UpgradeWrite struct {
	Method Method
	Store  string
	Args   []any
}

type UpgradeAbort struct{}

func (CreateStore) isUpgrade()  {}
func (DeleteStore) isUpgrade()  {}
func (CreateIndex) isUpgrade()  {}
func (DeleteIndex) isUpgrade()  {}
func (UpgradeWrite) isUpgrade() {}
func (UpgradeAbort) isUpgrade() {}

// contractError is an input outside the contract. run rejects with it as a TypeError.
type contractError string

func (e contractError) Error() string { return "outside the idb contract: " + string(e) }

// decode reads a script. The decoder panics with a contractError on an input outside the schema;
// decode returns it.
func decode(input *js.Object) (steps []Step, err error) {
	defer func() {
		if recovered := recover(); recovered != nil {
			invalid, ok := recovered.(contractError)
			if !ok {
				panic(recovered)
			}
			err = invalid
		}
	}()
	return decodeSteps(list(object(input), "steps")), nil
}

func object(value *js.Object) *js.Object {
	if kind(value) != "[object Object]" {
		panic(contractError("not an object"))
	}
	return value
}

func field(object *js.Object, name string) *js.Object {
	value := object.Get(name)
	if value == js.Undefined {
		panic(contractError("no " + name))
	}
	return value
}

func textOf(value *js.Object, what string) string {
	if kind(value) != "[object String]" {
		panic(contractError(what + " is not a string"))
	}
	return value.String()
}

func itemsOf(value *js.Object, what string) []*js.Object {
	if kind(value) != "[object Array]" {
		panic(contractError(what + " is not an array"))
	}
	return elements(value)
}

func text(object *js.Object, name string) string       { return textOf(field(object, name), name) }
func list(object *js.Object, name string) []*js.Object { return itemsOf(field(object, name), name) }

func flag(object *js.Object, name string) bool {
	value := field(object, name)
	if kind(value) != "[object Boolean]" {
		panic(contractError(name + " is not a boolean"))
	}
	return value.Bool()
}

// nameOrNull is a name, or nil for null.
func nameOrNull(object *js.Object, name string) *string {
	value := field(object, name)
	if value == nil {
		return nil
	}
	text := textOf(value, name)
	return &text
}

func method(name string) Method {
	switch m := Method(name); m {
	case Get, GetKey, GetAll, GetAllKeys, Count, Put, Add, Delete, Clear:
		return m
	}
	panic(contractError("not a request method: " + name))
}

// query is a query method's first argument: a key is passed as it is, null included; an object is
// a key range, its fields tested with `in` (the B9 decision: {only: null} is only(null)).
func query(value *js.Object) any {
	if kind(value) != "[object Object]" {
		return value
	}
	has := func(name string) bool { return value.Get(name) != js.Undefined }
	switch {
	case has("only"):
		return keyRange{"only", []any{value.Get("only")}}
	case has("lower") && has("upper"):
		return keyRange{"bound", []any{value.Get("lower"), value.Get("upper"), value.Get("lowerOpen"), value.Get("upperOpen")}}
	case has("lower"):
		return keyRange{"lowerBound", []any{value.Get("lower"), value.Get("lowerOpen")}}
	default:
		return keyRange{"upperBound", []any{value.Get("upper"), value.Get("upperOpen")}}
	}
}

// queryArgs reads a query method's arguments: the first is a query.
func queryArgs(values []*js.Object) []any {
	args := make([]any, len(values))
	for i, value := range values {
		if i == 0 {
			args[i] = query(value)
		} else {
			args[i] = value
		}
	}
	return args
}

// args reads a request's arguments: put and add take a value and a key, every other method a
// query first.
func args(m Method, values []*js.Object) []any {
	if m == Put || m == Add {
		return plain(values)
	}
	return queryArgs(values)
}

func plain(values []*js.Object) []any {
	args := make([]any, len(values))
	for i, value := range values {
		args[i] = value
	}
	return args
}

func target(object *js.Object) Target {
	return Target{Store: nameOrNull(object, "store"), Index: nameOrNull(object, "index")}
}

func request(object *js.Object, m Method) RequestStep {
	return RequestStep{Method: m, Target: target(object), Args: args(m, list(object, "args"))}
}

func action(value *js.Object) Action {
	items := itemsOf(value, "an action")
	if len(items) == 0 {
		panic(contractError("an empty action"))
	}
	name, args := ActionName(textOf(items[0], "an action")), items[1:]
	valid := false
	switch name {
	case ActionContinue:
		valid = len(args) <= 1
	case ActionAdvance, ActionUpdate:
		valid = len(args) == 1
	case ActionContinuePrimaryKey:
		valid = len(args) == 2
	case ActionDelete, ActionBreak:
		valid = len(args) == 0
	}
	if !valid {
		panic(contractError("not a cursor action: " + string(name)))
	}
	return Action{name, args}
}

func actions(value *js.Object) []Action {
	items := itemsOf(value, "a position")
	actions := make([]Action, len(items))
	for i, item := range items {
		actions[i] = action(item)
	}
	return actions
}

func walk(object *js.Object, kind WalkKind) WalkStep {
	positions := list(object, "positions")
	walk := WalkStep{Kind: kind, Target: target(object), Args: queryArgs(list(object, "args")), Rest: actions(field(object, "rest"))}
	for _, position := range positions {
		walk.Positions = append(walk.Positions, actions(position))
	}
	return walk
}

func blocked(step *js.Object) *Blocked {
	value := step.Get("blocked")
	switch {
	case value == js.Undefined:
		return nil
	case kind(value) == "[object String]" && value.String() == "log":
		return &Blocked{}
	default:
		conn := text(object(value), "close")
		return &Blocked{Close: &conn}
	}
}

func upgrade(value *js.Object) UpgradeStep {
	step := object(value)
	switch op := text(step, "op"); op {
	case "createStore":
		return CreateStore{Name: text(step, "name"), Options: step.Get("options")}
	case "deleteStore":
		return DeleteStore{Name: text(step, "name")}
	case "createIndex":
		return CreateIndex{Store: text(step, "store"), Name: text(step, "name"), KeyPath: field(step, "keyPath"), Options: step.Get("options")}
	case "deleteIndex":
		return DeleteIndex{Store: text(step, "store"), Name: text(step, "name")}
	case "put", "add":
		return UpgradeWrite{Method: Method(op), Store: text(step, "store"), Args: plain(list(step, "args"))}
	case "abort":
		return UpgradeAbort{}
	default:
		panic(contractError("not an upgrade step: " + op))
	}
}

func decodeSteps(values []*js.Object) []Step {
	steps := make([]Step, len(values))
	for i, value := range values {
		steps[i] = decodeStep(object(value))
	}
	return steps
}

func decodeStep(step *js.Object) Step {
	var op Op
	switch name := text(step, "op"); name {
	case "openDB":
		open := OpenDBStep{DB: text(step, "db"), Conn: text(step, "conn"), Version: step.Get("version"), Blocked: blocked(step)}
		if upgrades := step.Get("upgrade"); upgrades != js.Undefined {
			open.HasUpgrade = true
			for _, value := range itemsOf(upgrades, "upgrade") {
				open.Upgrade = append(open.Upgrade, upgrade(value))
			}
		}
		if blocking := step.Get("blocking"); blocking != js.Undefined {
			switch textOf(blocking, "blocking") {
			case "log":
				open.Blocking = &Blocking{}
			case "close":
				open.Blocking = &Blocking{Close: true}
			default:
				panic(contractError("blocking"))
			}
		}
		op = open
	case "close":
		op = CloseStep{Conn: text(step, "conn")}
	case "deleteDB":
		op = DeleteDBStep{DB: text(step, "db"), Blocked: blocked(step)}
	case "tx":
		op = TransactionStep{Conn: text(step, "conn"), Stores: field(step, "stores"), Mode: field(step, "mode"), Steps: decodeSteps(list(step, "steps"))}
	case "call":
		// database-extras' getMethod: prop.replace(/FromIndex$/, ''); only read methods have an
		// index form.
		label := text(step, "method")
		base := strings.TrimSuffix(label, "FromIndex")
		m := method(base)
		if base != label && m.isWrite() {
			panic(contractError("not a database shortcut: " + label))
		}
		op = ShortcutStep{Conn: text(step, "conn"), Label: label, Method: m, Store: text(step, "store"), Index: nameOrNull(step, "index"), Args: args(m, list(step, "args"))}
	case "batch":
		batch := BatchStep{Abort: flag(step, "abort"), Done: flag(step, "done")}
		for _, value := range list(step, "requests") {
			r := object(value)
			batch.Requests = append(batch.Requests, request(r, method(text(r, "op"))))
		}
		op = batch
	case "openCursor":
		op = walk(step, CursorWalk)
	case "openKeyCursor":
		op = walk(step, KeyCursorWalk)
	case "iterate":
		op = walk(step, IterateWalk)
	case "abort":
		op = AbortStep{}
	case "done":
		op = DoneStep{}
	case "store":
		op = StoreStep{}
	case "wrap":
		op = WrapStep{request(step, method(text(step, "method")))}
	default:
		op = request(step, method(name))
	}
	catch := false
	if value := step.Get("catch"); value != js.Undefined {
		catch = flag(step, "catch")
	}
	return Step{Op: op, Catch: catch}
}
