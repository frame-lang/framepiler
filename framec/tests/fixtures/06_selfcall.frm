@@system SelfCall {
    interface:
        kick()
        report(): i32

    machine:
        $Active {
            kick() {
                self.count = self.count + 1
                @@:self.report()
            }
            report(): i32 { @@:(self.count) }
        }

    domain:
        count: i32 = 0
}
