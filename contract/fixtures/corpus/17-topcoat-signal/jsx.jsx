// Family 17 -- Topcoat `signal name = init;` declarations, `$(sig.get())` reads
// and `@click=$(|_| sig.set(..))` writes.
// Solid's equivalent is createSignal plus a getter call in the reactive
// position and a setter call in the handler. The plugin does not see the signal
// declaration at all -- it only sees a call in a dynamic position -- which is
// exactly the delta this family is here to record.

const [count, setCount] = createSignal(0);

const counter = (
  <div>
    <button onClick={() => setCount(count() + 1)}>+1</button>
    <p>Count: {count()}</p>
  </div>
);

const [name, setName] = createSignal("");

const input = (
  <div>
    <input value={name()} onInput={e => setName(e.target.value)} />
    <p>Hello, {name()}!</p>
  </div>
);

const [open, setOpen] = createSignal(false);

const toggle = (
  <div>
    <button onClick={() => setOpen(!open())}>What is Topcoat?</button>
    <p hidden={!open()}>A fullstack Rust framework.</p>
  </div>
);
