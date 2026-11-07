// Stand-in for non-JS asset imports (CSS modules, images, fonts) that appear in
// a few upstream fixtures. A bundler would turn these into an object of class
// names or a URL string; nothing about them is observable in a runtime call
// trace, so an inert default is enough to let the module finish loading.
export default new Proxy({}, { get: (_, key) => (typeof key === "string" ? key : undefined) });
