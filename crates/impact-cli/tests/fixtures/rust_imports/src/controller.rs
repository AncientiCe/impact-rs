use crate::service::PaymentService;

pub struct Controller {
    pub service: PaymentService,
}

impl Controller {
    pub fn handle(&self) -> bool {
        self.service.charge()
    }
}
