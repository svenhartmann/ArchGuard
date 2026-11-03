package red;

import blue.Blue;

public class Red {
    public String describe() {
        return "red+" + new Blue().name();
    }
}
