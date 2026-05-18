@@system StateArgs {
    interface:
        load(initial: i32)
        adjust(delta: i32)
        peek(): i32

    machine:
        $Idle {
            load(initial: i32) { -> $Holding(initial) }
        }

        $Holding(value: i32) {
            adjust(delta: i32) { -> $Holding(value + delta) }
            peek(): i32 { @@:(value) }
        }
}
