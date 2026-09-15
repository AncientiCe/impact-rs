/// Same short name as `Widget::helper` — the ambiguity this fixture exists to rule out.
/// Nothing calls this one; it only exists so a bare call to `helper()` inside `Widget`
/// could, in principle, be misattributed to it too if the caller's own file scope didn't
/// correctly prefer the sibling method declared two lines away.
func helper() -> Bool {
    false
}
