// Family 15 -- Topcoat `:name=$(expr)` bind attributes.
// A bind attribute is an attribute kept in sync with a reactive expression, so
// the Solid equivalent is simply a dynamic attribute value: `hidden={...}`.
// The interesting part is which sink the plugin picks per name -- boolean
// attribute, property, className, style -- since Topcoat must pick the same one.

const bindHidden = <p hidden={!open()}>A fullstack Rust framework.</p>;

const bindValue = <input value={name()} />;

const bindChecked = <input type="checkbox" checked={done()} />;

const bindClass = <div class={cls()} />;

const bindStyle = <div style={style()} />;

const bindDisabled = <button disabled={busy()}>Save</button>;

const twoWay = (
  <div>
    <input value={name()} onInput={e => name.set(e.target.value)} />
    <p>Hello, {name()}!</p>
  </div>
);
