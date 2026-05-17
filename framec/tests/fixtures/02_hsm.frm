@@system MiniHsm {
    interface:
        wake()
        sleep()
        signal()

    machine:
        $Live {
            wake() { }
            sleep() { }
            signal() { }
        }

        $Awake => $Live {
            $>() { self.awakes = self.awakes + 1 }
            signal() {
                self.last = "awake"
                => $^
            }
        }

        $Asleep => $Live {
            $>() { self.sleeps = self.sleeps + 1 }
            signal() {
                self.last = "asleep"
                => $^
            }
            wake() { -> $Awake }
        }

    domain:
        awakes: i32 = 0
        sleeps: i32 = 0
        last: String = ""
}
