// Family 16 -- Topcoat attribute spreads, `<div (attrs)>`.
// The JSX equivalent is `{...attrs}`. The cases kept are the ones that decide
// whether the plugin can keep a template at all: a spread on its own, a spread
// mixed with static attributes before and after it, and a spread of a call.

const spreadOnly = <div {...attrs} />;

const spreadBefore = <div {...attrs} id="main" />;

const spreadAfter = <div id="main" {...attrs} />;

const spreadBetween = <div class="base" {...attrs} id="main" />;

const spreadCall = <div {...getProps("test")} />;

const spreadWithChildren = (
  <div {...attrs}>
    <span>Save</span>
  </div>
);

const spreadOnComponent = <Child {...props} name="John" />;
