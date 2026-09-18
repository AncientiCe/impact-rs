#include "util.h"

namespace outer {
namespace inner {

bool ns_caller() {
    return helper();
}

}
}
