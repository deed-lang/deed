"""Hold a real MCP session with `deed mcp` and assert on what comes back.

The crate's own tests speak to the server directly, which checks the protocol
this repository thinks it implements. This one goes through the reference
client, which is the protocol the clients that will actually call it implement.
Those agree right now and nothing was keeping them that way: framing, the
initialize response, the tool schemas and the content blocks can all drift into
a shape our tests still accept.

    python3 crates/deed-mcp/smoke.py target/release/deed
"""

import asyncio
import json
import sys

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

# Checks cleanly, states an obligation and carries a test, so one module
# proves every half an agent reads: the diagnostics, the tier the checker
# settled on, and a test that runs.
#
# The upper bound on `n` is not decoration. Without it `n + n` overflows near
# the top of the range and the `ensures` is not true for every `n > 0`, which
# is what the generated property found the first time this file ran one. It
# had been sitting here since the file was written, passing.
CLEAN = """module smoke

fn twice(n: Int) -> Int
    where n > 0, n < 1000000000
    ensures ok => result > n
{
    n + n
}

test "twice doubles" {
    assert twice(3) == 6
}
"""

# The call cannot be settled ahead of time, so this must come back `guarded`
# carrying a reason. An agent that cannot see the reason cannot act on it.
GUARDED = """module smoke

fn take(count: Int) -> Int
    where count > 0
{
    count
}

fn caller(n: Int) -> Int {
    take(n)
}
"""

WANTED = {
    "deed_check": "source",
    "deed_test": "source",
    "deed_run": "source",
    "deed_fmt": "source",
    "deed_fix": "source",
    "deed_explain": "code",
}


def lines(answer):
    out = []
    for item in answer.content:
        for line in getattr(item, "text", "").splitlines():
            if line.strip():
                out.append(json.loads(line))
    return out


def field(obj, *names):
    """The client SDK renamed these between majors; the protocol did not."""
    for name in names:
        if hasattr(obj, name):
            return getattr(obj, name)
    raise AssertionError(f"none of {names} on {type(obj).__name__}")


async def main(binary: str) -> int:
    params = StdioServerParameters(command=binary, args=["mcp"])
    async with stdio_client(params) as (read, write):
        async with ClientSession(read, write) as session:
            info = await session.initialize()
            server = field(info, "server_info", "serverInfo")
            version = field(info, "protocol_version", "protocolVersion")
            print(f"{server.name} {server.version}, {version}")

            assert info.instructions, "an agent arrives with no idea what Deed is"
            for name in WANTED:
                assert name in info.instructions, f"the handshake never mentions {name}"

            listed = await session.list_tools()
            offered = {t.name: t for t in listed.tools}
            assert set(offered) == set(WANTED), f"tools are {sorted(offered)}"
            for name, argument in WANTED.items():
                schema = field(offered[name], "input_schema", "inputSchema") or {}
                assert schema.get("required") == [argument], f"{name} requires {schema.get('required')}"
                assert offered[name].description, f"{name} arrives undescribed"

            checked = lines(await session.call_tool("deed_check", {"source": CLEAN}))
            assert not [x for x in checked if x["kind"] == "diagnostic"], checked
            tiers = {x["tier"] for x in checked if x["kind"] == "obligation"}
            # The call site is a literal, so the checker settles that one and
            # leaves the `ensures` to the test: two of the three tiers, and
            # `GUARDED` below carries the third.
            assert tiers == {"tested", "proven"}, tiers

            guarded = lines(await session.call_tool("deed_check", {"source": GUARDED}))
            settled = [x for x in guarded if x["kind"] == "obligation"]
            assert [x["tier"] for x in settled] == ["guarded"], settled
            assert settled[0]["reason"], "a guarded obligation says nothing about why"

            tested = lines(await session.call_tool("deed_test", {"source": CLEAN}))
            assert [x for x in tested if x["kind"] == "test" and x["passed"]], tested
            # The one nobody wrote. `deed test` runs it from the terminal, so a
            # surface that skipped it would answer a narrower question under
            # the same name.
            properties = [x for x in tested if x["kind"] == "property"]
            assert len(properties) == 1 and properties[0]["passed"], tested
            assert properties[0]["seed"], "a property run you cannot reproduce is a rumour"
            # The summary is what tells a client the difference between a file
            # whose tests all passed and a file with no tests in it. Without
            # it both answers are the same absence.
            summary = [x for x in tested if x["kind"] == "summary"]
            assert len(summary) == 1 and summary[0]["failed"] == 0, tested

            refused = lines(await session.call_tool("deed_test", {"source": "module x\n\nfn f() -> Int {\n    nonesuch\n}\n\ntest \"t\" {\n    assert 1 == 1\n}\n"}))
            assert [x["kind"] for x in refused] == ["refused"], refused

            page = lines(await session.call_tool("deed_explain", {"code": "DEED4025"}))
            assert page[0]["code"] == "DEED4025", page
            assert page[0]["text"], "the page for a code is empty"

            broken = lines(await session.call_tool("deed_check", {"source": "module x\n\nfn f() -> Int {\n"}))
            assert [x for x in broken if x["kind"] == "diagnostic"], "a broken module checked cleanly"

    print(f"a real client held a session and drove {len(WANTED)} tools")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main(sys.argv[1])))
