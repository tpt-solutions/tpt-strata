// Smoke test for the WebAssembly build, run under Node.js:
//   wasm-pack build crates/tpt-strata-wasm --target nodejs --out-dir ../../playground/pkg-node --release
//   node playground/test/smoke.cjs
const assert = require("node:assert/strict");
const wasm = require("../pkg-node/tpt_strata_wasm.js");

const tables = JSON.stringify({
  orders: "id,cust,amount\n1,1,2.5\n2,2,4\n3,1,\n",
  customers: "cid,name\n1,ann\n2,ben\n",
});

// Results come back as JSON text.
const result = JSON.parse(
  wasm.run(tables, "SELECT customers.name, SUM(orders.amount) AS total FROM orders " +
    "JOIN customers ON orders.cust = customers.cid GROUP BY customers.name ORDER BY name"),
);
assert.deepEqual(result.columns, ["name", "total"]);
assert.deepEqual(result.rows, [["ann", 2.5], ["ben", 4]]);

// Errors are thrown with a kind tag.
assert.throws(() => wasm.run(tables, "SELECT nope FROM orders"), /^Error: \[missing-column\]/);

// Text grid and plan.
assert.match(wasm.render_query(tables, "SELECT name FROM customers ORDER BY name"), /\| ann/);
assert.match(wasm.explain_query(tables, "SELECT id FROM orders WHERE amount > 1"), /Filter amount > 1/);

// check() passes silently for valid SQL and throws otherwise.
assert.equal(wasm.check_query(tables, "SELECT id FROM orders"), undefined);
assert.throws(() => wasm.check_query(tables, "SELECT id FROM nothere"));

console.log("wasm smoke test passed; engine version", wasm.version());
