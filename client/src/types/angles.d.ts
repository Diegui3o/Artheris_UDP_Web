export interface AnglesData {
  roll?: number;
  pitch?: number;
  yaw?: number;
  AngleRoll_est?: number;
  AnglePitch_est?: number;

  AngleRoll?: number;
  AnglePitch?: number;
  AngleYaw?: number;
  KalmanAngleRoll?: number;
  KalmanAnglePitch?: number;

  RateRoll?: number;
  RatePitch?: number;
  RateYaw?: number;
  GyroXdps?: number;
  GyroYdps?: number;
  GyroZdps?: number;
  gyroRateRoll?: number;
  gyroRatePitch?: number;
  gyroRateYaw?: number;
  
  DesiredAngleRoll?: number;
  DesiredAnglePitch?: number;
  DesiredRateYaw?: number;
  error_phi?: number;
  error_theta?: number;
  ErrorRoll?: number;
  ErrorPitch?: number;
  ErrorYaw?: number;

  AccX?: number;
  AccY?: number;
  AccZ?: number;

  tau_x?: number;
  tau_y?: number;
  tau_z?: number;
  InputThrottle?: number;
  InputRoll?: number;
  InputPitch?: number;
  InputYaw?: number;
  MotorInput1?: number;
  MotorInput2?: number;
  MotorInput3?: number;
  MotorInput4?: number;
  Altura?: number;
  modo?: string;
  modoActual?: string;
  k1?: number;
  k2?: number;
  k3?: number;
  g1?: number;
  g2?: number;
  g3?: number;
  m1?: number;
  m2?: number;
  m3?: number;
  time?: string;
}
