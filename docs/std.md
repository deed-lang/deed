# Standard library API

These pages are generated from the shipped module declarations under `std/` and the module tests that name each function. Each page records the declaration the compiler ships, the row variables and declared row it carries, and the example lines those tests exercise.

## Pages

- [`std/string`](std/string.md)
- [`std/list`](std/list.md)
- [`std/table`](std/table.md)
- [`std/map`](std/map.md)
- [`std/ratio`](std/ratio.md)
- [`std/date`](std/date.md)
- [`std/task`](std/task.md)

## User modules

`deed doc <path>...` writes the same page for any module, to standard output. There are no visibility modifiers here, so every declaration is API and there is nothing to decide about what belongs on a page. These pages are checked in only for the shipped modules, because those are the ones a reader has without having written them.
