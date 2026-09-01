use super::service::PaymentService;

pub struct PaymentController {
    pub service: PaymentService,
}

impl PaymentController {
    pub fn handle(&self) -> bool {
        self.service.charge()
    }
}
