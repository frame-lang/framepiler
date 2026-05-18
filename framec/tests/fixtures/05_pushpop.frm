@@system PushPop {
    interface:
        ping()
        nest()
        unnest()
        leaf()

    machine:
        $Idle {
            ping() { self.pings = self.pings + 1 }
            nest() {
                push$
                -> $Nested
            }
        }

        $Nested {
            leaf() { self.leaves = self.leaves + 1 }
            unnest() { -> pop$ }
        }

    domain:
        pings: i32 = 0
        leaves: i32 = 0
}
